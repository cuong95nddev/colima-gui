//! Background worker: talks to Colima (CLI) and Docker (socket) on a tokio runtime,
//! publishes snapshots into shared state, and executes commands sent from the UI.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::models::{ContainerSummary, ImageSummary, Volume};
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, LogsOptionsBuilder,
    PruneImagesOptions, PruneVolumesOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveImageOptions, RemoveVolumeOptionsBuilder, RestartContainerOptions,
    StartContainerOptions, StatsOptionsBuilder, StopContainerOptions,
};
use bollard::{API_DEFAULT_VERSION, Docker};
use eframe::egui;
use futures_util::StreamExt;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

pub const HISTORY_LEN: usize = 60;
const MAX_LOG_LINES: usize = 2000;

#[derive(Clone, Debug, Default)]
pub struct ColimaInfo {
    pub running: bool,
    pub cpu: u64,
    pub memory: u64,
    pub disk: u64,
    pub runtime: String,
    pub driver: String,
    pub arch: String,
    pub socket: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Stat {
    pub cpu: f64,
    pub mem: u64,
    pub mem_limit: u64,
    pub cpu_hist: VecDeque<f64>,
    pub mem_hist: VecDeque<f64>,
}

/// Which command-line tools are installed (checked periodically, so installing them
/// in Terminal is picked up without restarting the app).
#[derive(Clone, Copy, Debug, Default)]
pub struct Setup {
    pub checked: bool,
    pub colima: bool,
    pub docker: bool,
    pub brew: bool,
}

impl Setup {
    fn detect() -> Self {
        Setup { checked: true, colima: which("colima").is_some(), docker: which("docker").is_some(), brew: which("brew").is_some() }
    }

    /// Colima is known to be missing (not just "not checked yet").
    pub fn missing_colima(&self) -> bool {
        self.checked && !self.colima
    }
}

#[derive(Clone, Default)]
pub struct State {
    pub setup: Setup,
    pub colima: Option<ColimaInfo>,
    pub colima_busy: Option<String>,
    pub connected: bool,
    pub containers: Vec<ContainerSummary>,
    pub stats: HashMap<String, Stat>,
    pub images: Vec<ImageSummary>,
    pub volumes: Vec<Volume>,
    pub volumes_in_use: HashSet<String>,
    pub images_in_use: HashSet<String>,
    pub logs_for: Option<String>,
    pub logs: VecDeque<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Keys (container id / image id / volume name / "prune:*") with an operation in flight.
    pub busy: HashSet<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

#[derive(Debug)]
pub enum Command {
    StartColima,
    StopColima,
    Container(String, ContainerAction),
    RemoveImage(String),
    PruneImages,
    RemoveVolume(String),
    PruneVolumes,
    FollowLogs(Option<String>),
    Refresh,
}

pub type Shared = Arc<Mutex<State>>;

/// Spawn the backend thread. Returns the command sender.
pub fn spawn(state: Shared, ctx: egui::Context) -> mpsc::UnboundedSender<Command> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(Worker::new(state, ctx).run(rx));
    });
    tx
}

/// PATH for child processes: GUI apps launched from Finder get a minimal PATH.
fn child_path() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    format!("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:{base}")
}

/// Full path of an executable, searching Homebrew locations and the (extended) PATH.
pub fn which(name: &str) -> Option<String> {
    child_path()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join(name))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn find_bin(name: &str) -> String {
    which(name).unwrap_or_else(|| name.to_string())
}

fn default_socket() -> String {
    if let Ok(h) = std::env::var("DOCKER_HOST")
        && h.starts_with("unix://")
    {
        return h;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    format!("unix://{home}/.colima/default/docker.sock")
}

async fn colima_status() -> ColimaInfo {
    let out = tokio::process::Command::new(find_bin("colima"))
        .args(["status", "--json"])
        .env("PATH", child_path())
        .output()
        .await;
    let Ok(out) = out else {
        return ColimaInfo::default();
    };
    if !out.status.success() {
        return ColimaInfo::default();
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    ColimaInfo {
        running: true,
        cpu: v["cpu"].as_u64().unwrap_or(0),
        memory: v["memory"].as_u64().unwrap_or(0),
        disk: v["disk"].as_u64().unwrap_or(0),
        runtime: v["runtime"].as_str().unwrap_or("").to_string(),
        driver: v["driver"].as_str().unwrap_or("").to_string(),
        arch: v["arch"].as_str().unwrap_or("").to_string(),
        socket: v["docker_socket"].as_str().map(str::to_string),
    }
}

struct Worker {
    state: Shared,
    ctx: egui::Context,
    docker: Option<Docker>,
    stat_tasks: HashMap<String, JoinHandle<()>>,
    log_task: Option<JoinHandle<()>>,
    kick: Arc<Notify>,
}

impl Worker {
    fn new(state: Shared, ctx: egui::Context) -> Self {
        Self { state, ctx, docker: None, stat_tasks: HashMap::new(), log_task: None, kick: Arc::new(Notify::new()) }
    }

    fn update(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
        self.ctx.request_repaint();
    }

    fn set_error(&self, msg: impl Into<String>) {
        let msg = msg.into();
        self.update(|s| s.error = Some(msg));
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Command>) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        let mut n: u64 = 0;
        let kick = self.kick.clone();
        loop {
            tokio::select! {
                _ = kick.notified() => self.refresh(true).await,
                _ = tick.tick() => {
                    // Docker lists every 2 s, colima status + images/volumes every 6 s.
                    self.refresh(n % 3 == 0).await;
                    n += 1;
                }
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd).await,
                    None => return,
                },
            }
        }
    }

    async fn ensure_docker(&mut self, socket: Option<String>) -> Option<Docker> {
        if let Some(d) = &self.docker
            && d.ping().await.is_ok()
        {
            return Some(d.clone());
        }
        self.docker = None;
        let sock = socket.unwrap_or_else(default_socket);
        let d = Docker::connect_with_unix(&sock, 30, API_DEFAULT_VERSION).ok()?;
        d.ping().await.ok()?;
        self.docker = Some(d.clone());
        Some(d)
    }

    async fn refresh(&mut self, full: bool) {
        if full {
            let setup = Setup::detect();
            let info = if setup.colima { colima_status().await } else { ColimaInfo::default() };
            self.update(|s| {
                s.setup = setup;
                s.colima = Some(info);
            });
        }
        let socket = self.state.lock().unwrap().colima.as_ref().and_then(|c| c.socket.clone());
        let Some(docker) = self.ensure_docker(socket).await else {
            for (_, h) in self.stat_tasks.drain() {
                h.abort();
            }
            self.update(|s| {
                s.connected = false;
                s.containers.clear();
                s.stats.clear();
            });
            return;
        };

        let opts = ListContainersOptionsBuilder::new().all(true).build();
        let containers = match docker.list_containers(Some(opts)).await {
            Ok(c) => c,
            Err(e) => return self.set_error(format!("list containers: {e}")),
        };

        // Keep one stats stream per running container.
        let running: HashSet<String> = containers
            .iter()
            .filter(|c| c.state.map(|s| s.to_string() == "running").unwrap_or(false))
            .filter_map(|c| c.id.clone())
            .collect();
        self.stat_tasks.retain(|id, h| {
            let keep = running.contains(id) && !h.is_finished();
            if !keep {
                h.abort();
            }
            keep
        });
        for id in &running {
            if !self.stat_tasks.contains_key(id) {
                let h = tokio::spawn(stats_stream(docker.clone(), id.clone(), self.state.clone(), self.ctx.clone()));
                self.stat_tasks.insert(id.clone(), h);
            }
        }

        let volumes_in_use: HashSet<String> = containers
            .iter()
            .flat_map(|c| c.mounts.iter().flatten())
            .filter_map(|m| m.name.clone())
            .collect();
        let images_in_use: HashSet<String> =
            containers.iter().filter_map(|c| c.image_id.clone()).collect();

        let (images, volumes) = if full {
            let imgs = docker
                .list_images(Some(ListImagesOptionsBuilder::new().all(false).build()))
                .await
                .ok();
            let vols = docker
                .list_volumes(None::<bollard::query_parameters::ListVolumesOptions>)
                .await
                .ok()
                .and_then(|r| r.volumes);
            (imgs, vols)
        } else {
            (None, None)
        };

        self.update(|s| {
            s.connected = true;
            s.stats.retain(|id, _| running.contains(id));
            s.containers = containers;
            s.volumes_in_use = volumes_in_use;
            s.images_in_use = images_in_use;
            if let Some(i) = images {
                s.images = i;
            }
            if let Some(v) = volumes {
                s.volumes = v;
            }
        });
    }

    async fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::StartColima | Command::StopColima => {
                let start = matches!(cmd, Command::StartColima);
                let label = if start { "Starting Colima…" } else { "Stopping Colima…" };
                self.update(|s| s.colima_busy = Some(label.into()));
                let out = tokio::process::Command::new(find_bin("colima"))
                    .arg(if start { "start" } else { "stop" })
                    .env("PATH", child_path())
                    .output()
                    .await;
                let err = match out {
                    Ok(o) if o.status.success() => None,
                    Ok(o) => Some(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").to_string()),
                    Err(e) => Some(e.to_string()),
                };
                self.docker = None;
                self.update(|s| {
                    s.colima_busy = None;
                    if let Some(e) = err {
                        s.error = Some(format!("colima: {e}"));
                    }
                });
                self.refresh(true).await;
            }
            Command::FollowLogs(id) => {
                if let Some(h) = self.log_task.take() {
                    h.abort();
                }
                self.update(|s| {
                    s.logs_for = id.clone();
                    s.logs.clear();
                });
                if let (Some(id), Some(d)) = (id, self.docker.clone()) {
                    self.log_task = Some(tokio::spawn(logs_stream(d, id, self.state.clone(), self.ctx.clone())));
                }
            }
            Command::Refresh => self.refresh(true).await,
            other => {
                let Some(d) = self.docker.clone() else {
                    return self.set_error("Docker is not connected");
                };
                let key = match &other {
                    Command::Container(id, _) | Command::RemoveImage(id) | Command::RemoveVolume(id) => id.clone(),
                    Command::PruneImages => "prune:images".into(),
                    Command::PruneVolumes => "prune:volumes".into(),
                    _ => unreachable!(),
                };
                self.update(|s| {
                    s.busy.insert(key.clone());
                });
                // Run in its own task so a slow `docker stop` never blocks refreshes.
                let (state, ctx, kick) = (self.state.clone(), self.ctx.clone(), self.kick.clone());
                tokio::spawn(async move {
                    let res = run_op(d, other).await;
                    {
                        let mut s = state.lock().unwrap();
                        s.busy.remove(&key);
                        match res {
                            Ok(msg) => s.notice = Some(msg),
                            Err(e) => s.error = Some(e.to_string()),
                        }
                    }
                    ctx.request_repaint();
                    kick.notify_one();
                });
            }
        }
    }
}

async fn run_op(d: Docker, cmd: Command) -> Result<String, bollard::errors::Error> {
    match cmd {
        Command::Container(id, a) => match a {
            ContainerAction::Start => d.start_container(&id, None::<StartContainerOptions>).await.map(|_| "Container started".into()),
            ContainerAction::Stop => d.stop_container(&id, None::<StopContainerOptions>).await.map(|_| "Container stopped".into()),
            ContainerAction::Restart => d.restart_container(&id, None::<RestartContainerOptions>).await.map(|_| "Container restarted".into()),
            ContainerAction::Remove => d
                .remove_container(&id, Some(RemoveContainerOptionsBuilder::new().force(true).build()))
                .await
                .map(|_| "Container removed".into()),
        },
        Command::RemoveImage(id) => d.remove_image(&id, None::<RemoveImageOptions>, None).await.map(|_| "Image removed".into()),
        Command::PruneImages => d.prune_images(None::<PruneImagesOptions>).await.map(|r| {
            format!(
                "Pruned {} images, reclaimed {}",
                r.images_deleted.map(|v| v.len()).unwrap_or(0),
                crate::fmt_bytes(r.space_reclaimed.unwrap_or(0) as u64)
            )
        }),
        Command::RemoveVolume(name) => d
            .remove_volume(&name, Some(RemoveVolumeOptionsBuilder::new().force(false).build()))
            .await
            .map(|_| "Volume removed".into()),
        Command::PruneVolumes => {
            let filters = HashMap::from([("all".to_string(), vec!["true".to_string()])]);
            d.prune_volumes(Some(PruneVolumesOptionsBuilder::new().filters(&filters).build())).await.map(|r| {
                format!(
                    "Pruned {} volumes, reclaimed {}",
                    r.volumes_deleted.map(|v| v.len()).unwrap_or(0),
                    crate::fmt_bytes(r.space_reclaimed.unwrap_or(0) as u64)
                )
            })
        }
        _ => unreachable!(),
    }
}

async fn stats_stream(docker: Docker, id: String, state: Shared, ctx: egui::Context) {
    let mut stream = docker.stats(&id, Some(StatsOptionsBuilder::new().stream(true).build()));
    while let Some(Ok(st)) = stream.next().await {
        let cpu = (|| {
            let c = st.cpu_stats.as_ref()?;
            let p = st.precpu_stats.as_ref()?;
            let cd = c.cpu_usage.as_ref()?.total_usage? as f64 - p.cpu_usage.as_ref()?.total_usage.unwrap_or(0) as f64;
            let sd = c.system_cpu_usage? as f64 - p.system_cpu_usage.unwrap_or(0) as f64;
            let n = c.online_cpus.unwrap_or(1) as f64;
            (sd > 0.0 && cd >= 0.0).then(|| cd / sd * n * 100.0)
        })()
        .unwrap_or(0.0);
        let (mem, limit) = st
            .memory_stats
            .as_ref()
            .map(|m| {
                let cache = m.stats.as_ref().and_then(|s| s.get("inactive_file").or(s.get("cache")).copied()).unwrap_or(0);
                (m.usage.unwrap_or(0).saturating_sub(cache), m.limit.unwrap_or(0))
            })
            .unwrap_or((0, 0));
        {
            let mut s = state.lock().unwrap();
            let e = s.stats.entry(id.clone()).or_default();
            e.cpu = cpu;
            e.mem = mem;
            e.mem_limit = limit;
            e.cpu_hist.push_back(cpu);
            e.mem_hist.push_back(mem as f64 / 1_048_576.0);
            while e.cpu_hist.len() > HISTORY_LEN {
                e.cpu_hist.pop_front();
            }
            while e.mem_hist.len() > HISTORY_LEN {
                e.mem_hist.pop_front();
            }
        }
        ctx.request_repaint();
    }
}

async fn logs_stream(docker: Docker, id: String, state: Shared, ctx: egui::Context) {
    let opts = LogsOptionsBuilder::new().follow(true).stdout(true).stderr(true).tail("500").build();
    let mut stream = docker.logs(&id, Some(opts));
    while let Some(item) = stream.next().await {
        let Ok(out) = item else { break };
        let text = out.to_string();
        {
            let mut s = state.lock().unwrap();
            if s.logs_for.as_deref() != Some(id.as_str()) {
                return;
            }
            for line in text.lines() {
                s.logs.push_back(line.to_string());
            }
            while s.logs.len() > MAX_LOG_LINES {
                s.logs.pop_front();
            }
        }
        ctx.request_repaint();
    }
}
