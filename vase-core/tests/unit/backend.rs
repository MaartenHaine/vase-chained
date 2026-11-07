use crate::backend::*;
use crate::geometry::Rect;
use crate::tree::WindowId;

fn win(layer: i64, title: &str, w: f64, h: f64) -> WindowInfo {
    WindowInfo { id: WindowId(1), pid: 1, app: "App".into(), title: title.into(), frame: Rect::new(0.0, 0.0, w, h), layer }
}

#[test]
fn normal_titled_window_is_manageable() {
    assert!(manageable(&win(0, "Editor", 800.0, 600.0)));
}

#[test]
fn overlay_layer_and_untitled_and_tiny_are_not_manageable() {
    assert!(!manageable(&win(25, "Menu", 800.0, 600.0)));
    assert!(!manageable(&win(0, "", 800.0, 600.0)));
    assert!(!manageable(&win(0, "Tip", 10.0, 10.0)));
}

fn display(id: u32, x: f64, y: f64) -> Display {
    let bounds = Rect::new(x, y, 1920.0, 1080.0);
    Display { id, bounds, work_area: bounds }
}

#[test]
fn the_origin_display_is_the_one_at_zero_zero() {
    assert_eq!(origin_display(&[display(1, 0.0, 0.0), display(2, 1920.0, 0.0)]), 0);
    assert_eq!(origin_display(&[display(1, -1920.0, 0.0), display(2, 0.0, 0.0)]), 1);
}

#[test]
fn with_no_display_at_the_origin_the_first_is_taken() {
    assert_eq!(origin_display(&[display(1, 100.0, 0.0), display(2, 2020.0, 0.0)]), 0);
    assert_eq!(origin_display(&[]), 0);
}
