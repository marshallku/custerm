mod app;
mod terminal;
mod window;

fn main() {
    env_logger::init();
    app::run();
}
