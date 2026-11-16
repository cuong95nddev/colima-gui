//! Vertically centre the macOS traffic lights inside our taller custom top bar.
//! (With a hidden title bar AppKit keeps them at the default 28 pt title-bar height.)

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindowButton, NSWindowStyleMask};
use objc2_foundation::NSPoint;

/// Place the window buttons so they are centred in a bar `bar_height` pt tall, the first one
/// starting `left` pt from the window edge. Cheap and idempotent: call it every frame, since
/// AppKit resets the layout on resize and when leaving full screen.
pub fn center_traffic_lights(bar_height: f64, left: f64) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            continue;
        }
        let buttons = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
            .map(|b| window.standardWindowButton(b));
        let [Some(close), Some(mini), Some(zoom)] = buttons else { continue };
        // close button → NSTitlebarView → NSTitlebarContainerView
        let Some(container) = (unsafe { close.superview() }).and_then(|v| unsafe { v.superview() }) else { continue };

        let win_h = window.frame().size.height;
        let mut f = container.frame();
        if (f.size.height - bar_height).abs() > 0.5 || (f.origin.y - (win_h - bar_height)).abs() > 0.5 {
            f.size.height = bar_height;
            f.origin.y = win_h - bar_height;
            container.setFrame(f);
        }

        let btn_h = close.frame().size.height;
        let spacing = (mini.frame().origin.x - close.frame().origin.x).max(20.0);
        let y = ((bar_height - btn_h) / 2.0).round();
        for (i, b) in [close, mini, zoom].iter().enumerate() {
            let want = NSPoint::new(left + spacing * i as f64, y);
            let o = b.frame().origin;
            if (o.x - want.x).abs() > 0.5 || (o.y - want.y).abs() > 0.5 {
                b.setFrameOrigin(want);
            }
        }
    }
}
