mod allnotes;
mod app;
mod attachments;
mod backups;
mod card;
mod cli;
mod compositor;
mod crypto;
mod daily;
mod deck;
mod editor;
mod exporter;
mod markdown;
mod model;
mod paths;
mod prefs;
mod settings;
mod shortcuts;
mod signals;
mod store;
mod sync;
mod widgets;

fn main() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| cli::is_help(arg)) {
        print!("{}", cli::usage());
        return gtk::glib::ExitCode::SUCCESS;
    }
    if args.get(1).is_some_and(|arg| arg == "install-sway") {
        println!("{}", shortcuts::install_sway_bindings());
        return gtk::glib::ExitCode::SUCCESS;
    }
    app::run()
}
