#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod filesystem;
mod gui;
mod config;
mod history;
mod output;
mod platform;

fn main() -> eframe::Result {
    gui::run()
}
