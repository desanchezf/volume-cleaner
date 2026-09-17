mod filesystem;
mod gui;
mod config;
mod platform;

fn main() -> eframe::Result {
    gui::run()
}
