use adw::prelude::*;
use gtk::glib;

fn main() -> glib::ExitCode {
    adw::init().expect("libadwaita must initialize");

    let application = adw::Application::builder()
        .application_id("io.github.tannerkrewson.LiteBubbles")
        .build();

    application.connect_activate(|application| {
        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("LiteBubbles")
            .default_width(960)
            .default_height(640)
            .content(&gtk::Label::new(Some("LiteBubbles")))
            .build();
        window.present();
    });

    application.run()
}
