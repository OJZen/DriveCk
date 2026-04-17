#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The GTK frontend is only available on Linux.");
}

#[cfg(target_os = "linux")]
mod app {
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use driveck_core::{
        ProgressUpdate, TargetInfo, ValidationFailure, ValidationOptions, ValidationReport,
        collect_targets, discover_target, format_bytes, format_report_text, report_verdict,
        save_report, validate_target_with_callbacks,
    };
    use gtk::{
        Application, ApplicationWindow, Box as GtkBox, Button, DropDown, FileChooserAction,
        FileChooserNative, Label, MessageDialog, Orientation, ProgressBar, ResponseType,
        ScrolledWindow, StringList, TextBuffer, TextView,
        glib::{self, ControlFlow, Propagation},
        prelude::*,
    };

    #[derive(Debug)]
    enum WorkerMessage {
        Progress {
            phase: String,
            current: usize,
            total: usize,
        },
        Finished(WorkerResult),
    }

    #[derive(Debug)]
    struct WorkerResult {
        target: TargetInfo,
        report: Option<ValidationReport>,
        report_text: Option<String>,
        error: Option<String>,
    }

    struct AppState {
        window: ApplicationWindow,
        device_dropdown: DropDown,
        device_details_label: Label,
        refresh_button: Button,
        validate_button: Button,
        stop_button: Button,
        save_button: Button,
        status_label: Label,
        progress_bar: ProgressBar,
        report_buffer: TextBuffer,
        device_model: StringList,
        device_targets: Vec<TargetInfo>,
        report_text: Option<String>,
        last_target: Option<TargetInfo>,
        last_report: Option<ValidationReport>,
        worker: Option<JoinHandle<()>>,
        receiver: Option<mpsc::Receiver<WorkerMessage>>,
        cancel_requested: Arc<AtomicBool>,
        closing_requested: bool,
    }

    impl AppState {
        fn is_busy(&self) -> bool {
            self.worker.is_some()
        }

        fn set_status(&self, text: &str) {
            self.status_label.set_text(text);
        }

        fn set_report_text(&self, text: &str) {
            self.report_buffer.set_text(text);
        }

        fn update_actions(&self) {
            let busy = self.is_busy();
            let selected = self.device_dropdown.selected() as usize;
            let can_validate = !busy
                && self
                    .device_targets
                    .get(selected)
                    .is_some_and(|target| !target.is_mounted);

            self.device_dropdown
                .set_sensitive(!busy && !self.device_targets.is_empty());
            self.refresh_button.set_sensitive(!busy);
            self.validate_button.set_sensitive(can_validate);
            self.stop_button.set_sensitive(busy);
            self.save_button
                .set_sensitive(!busy && self.last_report.is_some() && self.report_text.is_some());
        }

        fn refresh_device_list(&mut self) {
            let targets = match collect_targets() {
                Ok(targets) => targets,
                Err(error) => {
                    self.show_message("Failed to refresh devices.", &error.message);
                    return;
                }
            };

            let previous_path = self
                .device_targets
                .get(self.device_dropdown.selected() as usize)
                .map(|target| target.path.clone());

            self.device_model
                .splice(0, self.device_model.n_items(), &[]);
            for target in &targets {
                self.device_model.append(&device_row_text(target));
            }

            self.device_targets = targets;
            if let Some(previous_path) = previous_path {
                if let Some(index) = self
                    .device_targets
                    .iter()
                    .position(|target| target.path == previous_path)
                {
                    self.device_dropdown.set_selected(index as u32);
                } else {
                    self.device_dropdown.set_selected(0);
                }
            } else {
                self.device_dropdown.set_selected(0);
            }
            self.update_device_details();
        }

        fn update_device_details(&self) {
            let selected = self.device_dropdown.selected() as usize;
            if let Some(target) = self.device_targets.get(selected) {
                self.device_details_label.set_text(&format!(
                    "Path: {}\nSize: {}\nTransport: {}{}\nModel: {}{}\nState: {}",
                    target.path,
                    format_bytes(target.size_bytes),
                    if target.is_usb { "usb" } else { "block" },
                    if target.is_removable {
                        ", removable"
                    } else {
                        ""
                    },
                    target.vendor,
                    if !target.model.is_empty() {
                        format!(" {}", target.model)
                    } else {
                        String::new()
                    },
                    if target.is_mounted {
                        "mounted, unmount before validating"
                    } else {
                        "ready"
                    }
                ));
            } else {
                self.device_details_label
                    .set_text("No removable or USB whole-disk device is currently available.");
            }
            self.update_actions();
        }

        fn prepare_selected_target(&self) -> Result<TargetInfo, String> {
            let selected = self.device_dropdown.selected() as usize;
            let target = self
                .device_targets
                .get(selected)
                .ok_or_else(|| "Choose a removable or USB device first.".to_string())?;
            let target = discover_target(&target.path).map_err(|error| error.message)?;
            if target.is_mounted {
                return Err(
                    "The selected disk is mounted. Unmount every partition before validating it."
                        .to_string(),
                );
            }
            Ok(target)
        }

        fn start_validation(&mut self, target: TargetInfo) {
            let (sender, receiver) = mpsc::channel::<WorkerMessage>();
            let cancel_requested = self.cancel_requested.clone();
            self.receiver = Some(receiver);
            self.cancel_requested.store(false, Ordering::Relaxed);
            self.closing_requested = false;
            self.report_text = None;
            self.last_report = None;
            self.last_target = None;
            self.set_report_text("Validation in progress...");
            self.set_status("Starting validation...");
            self.progress_bar.set_fraction(0.0);
            self.progress_bar.set_text(Some("Starting validation..."));
            self.update_actions();

            self.worker = Some(thread::spawn(move || {
                let mut progress = |update: ProgressUpdate| {
                    let _ = sender.send(WorkerMessage::Progress {
                        phase: update.phase.to_string(),
                        current: update.current,
                        total: update.total,
                    });
                };
                let cancel = || cancel_requested.load(Ordering::Relaxed);
                let result = validate_target_with_callbacks(
                    &target,
                    &ValidationOptions { seed: None },
                    Some(&mut progress),
                    Some(&cancel),
                );
                let worker_result = build_worker_result(target, result);
                let _ = sender.send(WorkerMessage::Finished(worker_result));
            }));
        }

        fn poll_worker_messages(&mut self) {
            let mut finished = None;
            if let Some(receiver) = self.receiver.as_ref() {
                while let Ok(message) = receiver.try_recv() {
                    match message {
                        WorkerMessage::Progress {
                            phase,
                            current,
                            total,
                        } => {
                            let fraction = if total == 0 {
                                0.0
                            } else {
                                current as f64 / total as f64
                            };
                            let text = format!("{phase} {current}/{total}");
                            self.progress_bar.set_fraction(fraction.min(1.0));
                            self.progress_bar.set_text(Some(&text));
                            self.set_status(&text);
                        }
                        WorkerMessage::Finished(result) => finished = Some(result),
                    }
                }
            }

            if let Some(result) = finished {
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                self.receiver = None;
                self.cancel_requested.store(false, Ordering::Relaxed);

                self.last_target = Some(result.target.clone());
                self.last_report = result.report.clone();
                self.report_text = result.report_text.clone();
                if let Some(text) = &self.report_text {
                    self.set_report_text(text);
                }

                if let Some(report) = &result.report {
                    let fraction = if report.completed_samples == 0 {
                        0.0
                    } else {
                        report.completed_samples as f64 / driveck_core::DRIVECK_SAMPLE_COUNT as f64
                    };
                    self.progress_bar.set_fraction(fraction);
                    let status_text = if let Some(error) = result.error.as_deref() {
                        if report.cancelled {
                            "Validation cancelled.".to_string()
                        } else {
                            error.to_string()
                        }
                    } else {
                        format!("Finished: {}", report_verdict(report))
                    };
                    self.progress_bar.set_text(Some(&status_text));
                    self.set_status(&status_text);
                } else if let Some(error) = result.error.as_deref() {
                    self.progress_bar.set_text(Some("Validation failed."));
                    self.set_status(error);
                    self.show_message("Validation failed.", error);
                }

                self.update_actions();
                if self.closing_requested {
                    if let Some(application) = self.window.application() {
                        application.quit();
                    }
                }
            }
        }

        fn show_message(&self, primary: &str, secondary: &str) {
            let dialog = MessageDialog::builder()
                .transient_for(&self.window)
                .modal(true)
                .text(primary)
                .secondary_text(secondary)
                .build();
            dialog.add_button("Close", ResponseType::Close);
            dialog.connect_response(|dialog, _| dialog.close());
            dialog.present();
        }
    }

    fn build_worker_result(
        target: TargetInfo,
        result: Result<ValidationReport, ValidationFailure>,
    ) -> WorkerResult {
        match result {
            Ok(report) => WorkerResult {
                report_text: Some(format_report_text(&target, &report)),
                target,
                report: Some(report),
                error: None,
            },
            Err(error) => {
                let report_text = error
                    .report
                    .as_ref()
                    .map(|report| format_report_text(&target, report));
                WorkerResult {
                    target,
                    report: error.report,
                    report_text,
                    error: Some(error.message),
                }
            }
        }
    }

    fn device_row_text(target: &TargetInfo) -> String {
        format!(
            "{}  {}  {}{}{}{}",
            target.path,
            format_bytes(target.size_bytes),
            target.vendor,
            if !target.vendor.is_empty() && !target.model.is_empty() {
                " "
            } else {
                ""
            },
            target.model,
            if target.is_mounted { "  [mounted]" } else { "" }
        )
    }

    fn build_ui(application: &Application) -> Rc<RefCell<AppState>> {
        let window = ApplicationWindow::builder()
            .application(application)
            .title("DriveCk")
            .default_width(960)
            .default_height(760)
            .build();

        let root = GtkBox::new(Orientation::Vertical, 12);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let intro = Label::new(Some(
            "Rust port of DriveCk.\nPick a removable or USB whole-disk target, then run a 576-point write/read/restore validation pass.",
        ));
        intro.set_wrap(true);
        intro.set_xalign(0.0);
        root.append(&intro);

        let device_model = StringList::new(&[]);
        let device_dropdown = DropDown::new(Some(device_model.clone()), None::<gtk::Expression>);
        let device_details_label = Label::new(None);
        device_details_label.set_wrap(true);
        device_details_label.set_xalign(0.0);
        root.append(&device_dropdown);
        root.append(&device_details_label);

        let actions = GtkBox::new(Orientation::Horizontal, 6);
        let refresh_button = Button::with_label("Refresh devices");
        let validate_button = Button::with_label("Validate");
        let stop_button = Button::with_label("Stop");
        let save_button = Button::with_label("Save report…");
        actions.append(&refresh_button);
        actions.append(&validate_button);
        actions.append(&stop_button);
        actions.append(&save_button);
        root.append(&actions);

        let status_label = Label::new(Some("Ready."));
        status_label.set_xalign(0.0);
        root.append(&status_label);

        let progress_bar = ProgressBar::new();
        progress_bar.set_show_text(true);
        progress_bar.set_text(Some("Idle"));
        root.append(&progress_bar);

        let scroller = ScrolledWindow::new();
        scroller.set_vexpand(true);
        let text_view = TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_monospace(true);
        let report_buffer = text_view.buffer();
        scroller.set_child(Some(&text_view));
        root.append(&scroller);

        window.set_child(Some(&root));

        let state = Rc::new(RefCell::new(AppState {
            window,
            device_dropdown,
            device_details_label,
            refresh_button,
            validate_button,
            stop_button,
            save_button,
            status_label,
            progress_bar,
            report_buffer,
            device_model,
            device_targets: Vec::new(),
            report_text: None,
            last_target: None,
            last_report: None,
            worker: None,
            receiver: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            closing_requested: false,
        }));

        state.borrow().set_report_text(
            "No validation has run yet.\n\nChoose a removable or USB whole-disk device that is fully unmounted, then start validation.",
        );

        {
            let state = state.clone();
            let button = state.borrow().refresh_button.clone();
            button.connect_clicked(move |_| state.borrow_mut().refresh_device_list());
        }
        {
            let state = state.clone();
            let button = state.borrow().validate_button.clone();
            button.connect_clicked(move |_| {
                let target = match state.borrow().prepare_selected_target() {
                    Ok(target) => target,
                    Err(error) => {
                        state.borrow().show_message("Cannot start validation.", &error);
                        return;
                    }
                };

                let state_for_response = state.clone();
                let dialog = MessageDialog::builder()
                    .transient_for(&state.borrow().window)
                    .modal(true)
                    .text(format!("Validate block device {}?", target.path))
                    .secondary_text(format!(
                        "DriveCk will temporarily overwrite sampled regions on the selected disk and then restore them. Only continue when you are certain that {} ({}, {}{}) is the correct target.",
                        target.path,
                        format_bytes(target.size_bytes),
                        target.vendor,
                        if target.model.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", target.model)
                        }
                    ))
                    .build();
                dialog.add_button("Cancel", ResponseType::Cancel);
                dialog.add_button("Validate", ResponseType::Accept);
                dialog.connect_response(move |dialog, response| {
                    if response == ResponseType::Accept {
                        state_for_response
                            .borrow_mut()
                            .start_validation(target.clone());
                    }
                    dialog.close();
                });
                dialog.present();
            });
        }
        {
            let state = state.clone();
            let button = state.borrow().stop_button.clone();
            button.connect_clicked(move |_| {
                let state = state.borrow();
                if state.is_busy() {
                    state.cancel_requested.store(true, Ordering::Relaxed);
                    state.set_status("Stopping...");
                    state.progress_bar.set_text(Some("Stopping..."));
                }
            });
        }
        {
            let state = state.clone();
            let button = state.borrow().save_button.clone();
            let window = state.borrow().window.clone();
            button.connect_clicked(move |_| {
                let (target, report) = {
                    let state = state.borrow();
                    match (state.last_target.clone(), state.last_report.clone()) {
                        (Some(target), Some(report)) => (target, report),
                        _ => return,
                    }
                };

                let dialog = FileChooserNative::builder()
                    .title("Save validation report")
                    .transient_for(&window)
                    .action(FileChooserAction::Save)
                    .accept_label("Save")
                    .cancel_label("Cancel")
                    .modal(true)
                    .build();
                let state = state.clone();
                dialog.connect_response(move |dialog, response| {
                    if response == ResponseType::Accept {
                        if let Some(file) = dialog.file() {
                            if let Some(path) = file.path() {
                                if let Err(error) = save_report(&path, &target, &report) {
                                    state
                                        .borrow()
                                        .show_message("Failed to save report.", &error.message);
                                } else {
                                    state.borrow().set_status("Report saved.");
                                }
                            }
                        }
                    }
                    dialog.destroy();
                });
                dialog.show();
            });
        }
        {
            let state = state.clone();
            let dropdown = state.borrow().device_dropdown.clone();
            dropdown.connect_selected_notify(move |_| {
                if let Ok(state) = state.try_borrow() {
                    state.update_device_details();
                }
            });
        }
        {
            let state = state.clone();
            let window = state.borrow().window.clone();
            window.connect_close_request(move |_| {
                let mut state = state.borrow_mut();
                if !state.is_busy() {
                    return Propagation::Proceed;
                }
                state.closing_requested = true;
                state.cancel_requested.store(true, Ordering::Relaxed);
                state.set_status("Stopping before exit...");
                state.progress_bar.set_text(Some("Stopping before exit..."));
                Propagation::Stop
            });
        }
        {
            let state = state.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                state.borrow_mut().poll_worker_messages();
                ControlFlow::Continue
            });
        }
        {
            let state = state.clone();
            glib::timeout_add_seconds_local(2, move || {
                if !state.borrow().is_busy() {
                    state.borrow_mut().refresh_device_list();
                }
                ControlFlow::Continue
            });
        }

        state.borrow_mut().refresh_device_list();
        state.borrow().window.present();
        state
    }

    pub fn run() {
        let application = Application::builder()
            .application_id("com.github.driveck")
            .build();
        application.connect_activate(|application| {
            let _state = build_ui(application);
        });
        application.run();
    }
}

#[cfg(target_os = "linux")]
fn main() {
    app::run();
}
