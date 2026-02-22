use slint::{CloseRequestResponse, ComponentHandle};

slint::slint! {
    import { Button, VerticalBox } from "std-widgets.slint";

    export component MinimizeDemo inherits Window {
        title: "slint-layer-shell minimize demo";
        width: 460px;
        height: 260px;

        in-out property <string> status_text: "Click a button to test minimize/restore/close.";

        callback request_minimize();
        callback request_restore();
        callback request_close();

        VerticalBox {
            spacing: 10px;

            Text {
                text: root.status_text;
                wrap: word-wrap;
            }

            Button {
                text: "Minimize";
                clicked => { root.request_minimize(); }
            }

            Button {
                text: "Restore";
                clicked => { root.request_restore(); }
            }

            Button {
                text: "Close";
                clicked => { root.request_close(); }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::platform::set_platform(Box::new(slint_layer_shell::SlintLayerShell::new()))?;

    let ui = MinimizeDemo::new()?;

    ui.window().on_close_requested(|| {
        println!("Close requested from compositor.");
        CloseRequestResponse::HideWindow
    });

    let weak = ui.as_weak();
    {
        let weak = weak.clone();
        ui.on_request_minimize(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_status_text("Sent set_minimized(true) to Slint Window.".into());
                ui.window().set_minimized(true);
            }
        });
    }

    {
        let weak = weak.clone();
        ui.on_request_restore(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_status_text("Sent set_minimized(false) to Slint Window.".into());
                ui.window().set_minimized(false);
            }
        });
    }

    ui.on_request_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_text("Calling hide() to close this demo window.".into());
            if let Err(err) = ui.hide() {
                eprintln!("hide() failed: {err}");
            }
        }
    });

    ui.run()?;
    Ok(())
}
