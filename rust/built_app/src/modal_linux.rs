//! Winit owns the Wayland/X11 connection. A synchronous question cannot
//! re-enter its event loop. Present it on the GTK thread that owns our
//! desktop clipboard and deliver the chosen button back to the same sheet.
use crate::native::Input;
use gtk::prelude::*;
use pyo3::prelude::*;

pub fn pump(py: Python<'_>, view: usize, timeout_ms: f64) -> PyResult<Vec<Input>> {
    let modal = py.import("app_shell.modal")?;
    let host = modal.call_method0("host")?;
    let win = host.getattr("window")?;
    let dialogs = py.import("views.chrome_dialogs")?;
    let sheet = dialogs.call_method1("current_sheet", (&win,))?;
    if sheet.is_none()
        || !sheet.getattr("blocking")?.is_truthy()?
        || !sheet.getattr("answer")?.is_none()
    {
        py.detach(|| {
            std::thread::sleep(std::time::Duration::from_millis(
                timeout_ms.clamp(0., 16.) as u64
            ))
        });
        #[cfg(target_os = "linux")]
        {
            let scale = host.getattr("scale")?.extract()?;
            let mods = host.getattr("input_state")?.getattr("modifiers")?.extract()?;
            return Ok(crate::linux_pointer::progress(view, scale, mods));
        }
        #[cfg(not(target_os = "linux"))]
        return Ok(Vec::new());
    }
    // Scripted answers are sent after the sheet has painted a few frames.
    if std::env::var_os("BUILT_ANSWERS").is_some() {
        return Ok(Vec::new());
    }
    if std::env::var_os("BUILT_BACKGROUND").is_some_and(|v| v != "0") {
        sheet.getattr("event_loop")?.call_method0("quit")?;
        return Ok(Vec::new());
    }
    let title: String = sheet.getattr("title")?.extract()?;
    let text: String = sheet.getattr("text")?.extract()?;
    let buttons: Vec<(String, String, String, i32)> = sheet.getattr("buttons")?.extract()?;
    let checks: Option<Vec<(String, bool)>> = sheet.getattr("checklist")?.extract()?;
    let checks = checks.unwrap_or_default();
    let labels = buttons.clone();
    let response = py.detach(move || {
        crate::clipboard::on_gtk_thread(move || {
            let dialog = gtk::Dialog::builder().title(&title).modal(true).build();
            let content = dialog.content_area();
            let message = gtk::Label::new(Some(&text));
            message.set_line_wrap(true);
            content.add(&message);
            let mut toggles = Vec::new();
            for (label, checked) in checks {
                let toggle = gtk::CheckButton::with_label(&label);
                toggle.set_active(checked);
                content.add(&toggle);
                toggles.push((toggle, checked));
            }
            for (index, (_, label, _, role)) in labels.iter().enumerate() {
                let id = gtk::ResponseType::Other(index as u16);
                dialog.add_button(label, id);
                if *role == 1 {
                    dialog.set_default_response(id);
                }
            }
            dialog.show_all();
            let response = dialog.run();
            let changed: Vec<_> = toggles
                .iter()
                .enumerate()
                .filter(|(_, (toggle, old))| toggle.is_active() != *old)
                .map(|(i, _)| i)
                .collect();
            dialog.close();
            (response, changed)
        })
    });
    let mut choice = sheet.getattr("cancel")?.extract::<Option<String>>()?;
    if let Some((response, changed)) = response {
        for index in changed {
            dialogs.call_method1("answer", (&win, format!("check:{index}")))?;
        }
        if let gtk::ResponseType::Other(index) = response {
            choice = buttons.get(index as usize).map(|b| b.0.clone());
        }
    }
    if let Some(id) = choice {
        dialogs.call_method1("answer", (&win, id))?;
    } else {
        sheet.getattr("event_loop")?.call_method0("quit")?;
    }
    Ok(Vec::new())
}

pub fn post_key(py: Python<'_>, view: usize, code: u16, command: bool) -> PyResult<bool> {
    let role = match (code, command) {
        (36, false) => "default",
        (53, false) => "cancel",
        (2, true) => "discard",
        _ => return Ok(false),
    };
    let host = py.import("app_shell.modal")?.call_method0("host")?;
    if host.getattr("ns_view")?.extract::<usize>()? != view {
        return Ok(false);
    }
    let win = host.getattr("window")?;
    let dialogs = py.import("views.chrome_dialogs")?;
    let sheet = dialogs.call_method1("current_sheet", (&win,))?;
    if sheet.is_none() {
        return Ok(false);
    }
    let button = sheet.getattr(role)?;
    if button.is_none() {
        return Ok(false);
    }
    dialogs.call_method1("answer", (win, button))?.extract()
}
