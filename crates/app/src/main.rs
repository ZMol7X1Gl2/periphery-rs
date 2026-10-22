#![windows_subsystem = "windows"]

//! Periphery desktop app. The window lives in [`periphery::ui`].

use eframe::egui;
use periphery::ui::{App, StartOptions};

fn main() -> eframe::Result {
    let start = StartOptions::from_args(std::env::args());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 820.0]).with_min_inner_size([820.0, 520.0]).with_icon(periphery::theme::window_icon()),
        ..Default::default()
    };
    eframe::run_native("Periphery", options, Box::new(|cc| Ok(Box::new(App::new(cc, start)))))
}
