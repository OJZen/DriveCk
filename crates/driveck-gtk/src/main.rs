#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The GTK frontend is only available on Linux.");
}

#[cfg(target_os = "linux")]
mod app {
    use std::{
        cell::RefCell,
        env,
        io::{self, BufRead, BufReader, BufWriter, Read as _, Write as _},
        os::unix::process::ExitStatusExt,
        process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
        rc::Rc,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use driveck_core::{
        collect_targets, discover_target, format_bytes, format_report_text, report_verdict,
        save_report, validate_target_with_callbacks, ProgressUpdate, SampleStatus, TargetInfo,
        ValidationFailure, ValidationOptions, ValidationReport, DRIVECK_MAP_COLUMNS,
        DRIVECK_MAP_ROWS,
    };
    use gtk::{
        gdk,
        glib::{self, ControlFlow, Propagation},
        prelude::*,
        style_context_add_provider_for_display, Align, Application, ApplicationWindow, AspectFrame,
        Box as GtkBox, Button, CssProvider, Dialog, DrawingArea, DropDown, FileChooserAction,
        FileChooserNative, Grid, Label, LinkButton, MessageDialog, Orientation, ResponseType,
        ScrolledWindow, StringList, TextView, STYLE_PROVIDER_PRIORITY_APPLICATION,
    };
    use serde::{Deserialize, Serialize};

    const GRID_ROWS: usize = DRIVECK_MAP_ROWS;
    const GRID_COLUMNS: usize = DRIVECK_MAP_COLUMNS;
    const GRID_HEIGHT: i32 = 236;
    const GRID_GAP: f64 = 1.0;
    const GRID_PADDING: f64 = 6.0;
    const APP_VERSION: &str = "v1.0";
    const PROJECT_URL: &str = "https://github.com/OJZen/DriveCk";
    const APP_CSS: &str = r#"
    .window-root {
        background: #f4f7fb;
    }

    .panel {
        background: #ffffff;
        border-radius: 16px;
        border: 1px solid rgba(15, 23, 42, 0.08);
        padding: 12px;
    }

    .app-title {
        font-size: 24px;
        font-weight: 800;
        color: #101828;
    }

    .app-subtitle,
    .device-meta,
    .device-path,
    .report-hint {
        color: #667085;
    }

    .panel-title {
        font-size: 14px;
        font-weight: 700;
        color: #344054;
    }

    .device-title {
        font-size: 16px;
        font-weight: 700;
        color: #101828;
    }

    .status-line {
        font-size: 13px;
        font-weight: 700;
        color: #344054;
    }

    .metric-chip,
    .legend-chip {
        border-radius: 999px;
        padding: 6px 10px;
        font-weight: 700;
    }

    .metric-neutral,
    .legend-pending {
        background: #eef2f6;
        color: #475467;
    }

    .metric-success,
    .legend-success {
        background: #e7f6ec;
        color: #167944;
    }

    .metric-danger,
    .legend-danger {
        background: #fdebec;
        color: #c4323f;
    }

    .summary-key {
        color: #667085;
        font-size: 12px;
        font-weight: 700;
    }

    .summary-value {
        color: #101828;
        font-size: 13px;
        font-weight: 700;
    }

    .version-link {
        color: #98a2b3;
        font-size: 12px;
        font-weight: 600;
        padding: 0;
    }

    .version-link:hover {
        color: #667085;
    }
    "#;

    #[derive(Debug)]
    enum WorkerMessage {
        Authenticated,
        Progress {
            phase: String,
            current: usize,
            total: usize,
            sample_index: Option<usize>,
            sample_status: Option<SampleStatus>,
            final_update: bool,
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

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum HelperEvent {
        Started,
        Progress {
            phase: String,
            current: usize,
            total: usize,
            sample_index: Option<usize>,
            sample_status: Option<SampleStatus>,
            final_update: bool,
        },
        Finished {
            report: Option<ValidationReport>,
            error: Option<String>,
        },
    }

    struct StartedValidation {
        receiver: mpsc::Receiver<WorkerMessage>,
        worker: JoinHandle<()>,
        helper_cancel_pipe: Option<Arc<Mutex<ChildStdin>>>,
        helper_process: Option<Arc<Mutex<Child>>>,
        auth_pending: bool,
    }

    pub enum LaunchMode {
        App,
        ValidateHelper { device_path: String },
    }

    #[derive(Debug, Clone)]
    struct ValidationGridState {
        sample_status: Vec<SampleStatus>,
        last_sample: Option<usize>,
    }

    impl Default for ValidationGridState {
        fn default() -> Self {
            Self {
                sample_status: vec![SampleStatus::Untested; driveck_core::DRIVECK_SAMPLE_COUNT],
                last_sample: None,
            }
        }
    }

    impl ValidationGridState {
        fn reset(&mut self) {
            self.sample_status.fill(SampleStatus::Untested);
            self.last_sample = None;
        }

        fn mark(&mut self, sample_index: usize, status: SampleStatus) {
            if let Some(slot) = self.sample_status.get_mut(sample_index) {
                *slot = status;
                self.last_sample = Some(sample_index);
            }
        }

        fn sync_from_report(&mut self, report: &ValidationReport) {
            self.sample_status = report.sample_status.clone();
            self.last_sample = None;
        }

        fn counts(&self) -> (usize, usize, usize) {
            let processed = self
                .sample_status
                .iter()
                .filter(|status| **status != SampleStatus::Untested)
                .count();
            let ok = self
                .sample_status
                .iter()
                .filter(|status| **status == SampleStatus::Ok)
                .count();
            let failed = processed.saturating_sub(ok);
            (processed, ok, failed)
        }
    }

    struct AppState {
        window: ApplicationWindow,
        device_dropdown: DropDown,
        device_title_label: Label,
        device_size_label: Label,
        device_transport_label: Label,
        device_state_label: Label,
        processed_label: Label,
        ok_label: Label,
        fail_label: Label,
        refresh_button: Button,
        action_button: Button,
        report_button: Button,
        status_label: Label,
        validation_map: DrawingArea,
        validation_grid_state: Rc<RefCell<ValidationGridState>>,
        device_model: StringList,
        device_targets: Vec<TargetInfo>,
        report_text: Option<String>,
        last_target: Option<TargetInfo>,
        last_report: Option<ValidationReport>,
        worker: Option<JoinHandle<()>>,
        receiver: Option<mpsc::Receiver<WorkerMessage>>,
        cancel_requested: Arc<AtomicBool>,
        helper_cancel_pipe: Option<Arc<Mutex<ChildStdin>>>,
        helper_process: Option<Arc<Mutex<Child>>>,
        helper_auth_pending: bool,
        confirmation_pending: bool,
        stop_requested: bool,
        closing_requested: bool,
    }

    impl AppState {
        fn is_busy(&self) -> bool {
            self.worker.is_some()
        }

        fn set_status(&self, text: &str) {
            self.status_label.set_text(text);
        }

        fn update_report_button(&self) {
            self.report_button
                .set_sensitive(!self.is_busy() && self.report_text.is_some());
        }

        fn refresh_live_metrics(&self) {
            let (processed, ok, failed) = self.validation_grid_state.borrow().counts();
            self.processed_label.set_text(&format!(
                "Done {processed}/{}",
                driveck_core::DRIVECK_SAMPLE_COUNT
            ));
            self.ok_label.set_text(&format!("OK {ok}"));
            self.fail_label.set_text(&format!("Fail {failed}"));

            self.fail_label.remove_css_class("metric-neutral");
            self.fail_label.remove_css_class("metric-danger");
            self.fail_label.add_css_class(if failed == 0 {
                "metric-neutral"
            } else {
                "metric-danger"
            });
        }

        fn reset_validation_view(&mut self) {
            self.report_text = None;
            self.last_report = None;
            self.last_target = None;
            self.set_status("Starting validation...");
            {
                let mut grid = self.validation_grid_state.borrow_mut();
                grid.reset();
            }
            self.validation_map.queue_draw();
            self.refresh_live_metrics();
        }

        fn request_stop(&self) {
            self.cancel_requested.store(true, Ordering::Relaxed);
            if self.helper_auth_pending {
                if let Some(helper_process) = &self.helper_process {
                    if let Ok(mut child) = helper_process.lock() {
                        let _ = child.kill();
                    }
                }
                return;
            }

            if let Some(cancel_pipe) = &self.helper_cancel_pipe {
                if let Ok(mut stdin) = cancel_pipe.lock() {
                    let _ = stdin.write_all(b"cancel\n");
                    let _ = stdin.flush();
                }
            }
        }

        fn update_actions(&self) {
            let busy = self.is_busy();
            let selected = self.device_dropdown.selected() as usize;
            let can_validate = !busy
                && !self.confirmation_pending
                && self
                    .device_targets
                    .get(selected)
                    .is_some_and(|target| !target.is_mounted);

            self.device_dropdown.set_sensitive(
                !busy && !self.confirmation_pending && !self.device_targets.is_empty(),
            );
            self.refresh_button
                .set_sensitive(!busy && !self.confirmation_pending);
            self.action_button.remove_css_class("suggested-action");
            self.action_button.remove_css_class("destructive-action");
            if busy {
                self.action_button.set_label(if self.stop_requested {
                    "Stopping..."
                } else {
                    "Stop"
                });
                self.action_button.add_css_class("destructive-action");
                self.action_button.set_sensitive(!self.stop_requested);
            } else {
                self.action_button.set_label("Validate");
                self.action_button.add_css_class("suggested-action");
                self.action_button.set_sensitive(can_validate);
            }
            self.update_report_button();
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
                self.device_title_label
                    .set_text(&device_display_name(target));
                self.device_size_label
                    .set_text(&format_bytes(target.size_bytes));
                self.device_transport_label
                    .set_text(device_transport_text(target));
                self.device_state_label.set_text(if target.is_mounted {
                    "Mounted"
                } else {
                    "Ready"
                });
                apply_chip_variant(
                    &self.device_transport_label,
                    if target.is_usb || target.is_removable {
                        "metric-success"
                    } else {
                        "metric-neutral"
                    },
                );
                apply_chip_variant(
                    &self.device_state_label,
                    if target.is_mounted {
                        "metric-danger"
                    } else {
                        "metric-success"
                    },
                );
            } else {
                self.device_title_label.set_text("No device available");
                self.device_size_label.set_text("No device");
                self.device_transport_label.set_text("USB / Removable");
                self.device_state_label.set_text("Waiting");
                apply_chip_variant(&self.device_transport_label, "metric-neutral");
                apply_chip_variant(&self.device_state_label, "metric-neutral");
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
            self.cancel_requested.store(false, Ordering::Relaxed);
            self.confirmation_pending = false;
            self.stop_requested = false;
            self.closing_requested = false;
            let started = if needs_privileged_helper() {
                spawn_privileged_validation_worker(target)
            } else {
                Ok(spawn_local_validation_worker(
                    target,
                    self.cancel_requested.clone(),
                ))
            };

            match started {
                Ok(started) => {
                    self.receiver = Some(started.receiver);
                    self.worker = Some(started.worker);
                    self.helper_cancel_pipe = started.helper_cancel_pipe;
                    self.helper_process = started.helper_process;
                    self.helper_auth_pending = started.auth_pending;
                    self.reset_validation_view();
                    if self.helper_auth_pending {
                        self.set_status("Waiting for administrator authentication...");
                    }
                    self.update_actions();
                }
                Err(error) => {
                    self.cancel_requested.store(false, Ordering::Relaxed);
                    self.helper_cancel_pipe = None;
                    self.helper_process = None;
                    self.helper_auth_pending = false;
                    self.stop_requested = false;
                    self.show_message("Cannot start validation.", &error);
                    self.update_actions();
                }
            }
        }

        fn poll_worker_messages(&mut self) {
            let mut finished = None;
            if let Some(receiver) = self.receiver.as_ref() {
                while let Ok(message) = receiver.try_recv() {
                    match message {
                        WorkerMessage::Authenticated => {
                            self.helper_auth_pending = false;
                            self.set_status("Administrator access granted. Starting validation...");
                        }
                        WorkerMessage::Progress {
                            phase,
                            current,
                            total,
                            sample_index,
                            sample_status,
                            final_update,
                        } => {
                            self.helper_auth_pending = false;
                            if let (Some(sample_index), Some(sample_status)) =
                                (sample_index, sample_status)
                            {
                                self.validation_grid_state
                                    .borrow_mut()
                                    .mark(sample_index, sample_status);
                                self.validation_map.queue_draw();
                                self.refresh_live_metrics();
                            }

                            let progress_text = format!("{current}/{total}");
                            let status_text = if final_update {
                                format!("{phase} {progress_text}")
                            } else {
                                format!("{phase} sample {progress_text}")
                            };
                            self.set_status(&status_text);
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
                self.helper_cancel_pipe = None;
                self.helper_process = None;
                self.helper_auth_pending = false;
                self.confirmation_pending = false;
                self.stop_requested = false;

                self.last_target = Some(result.target.clone());
                self.last_report = result.report.clone();
                self.report_text = result.report_text.clone();
                self.update_report_button();

                if let Some(report) = &result.report {
                    self.validation_grid_state
                        .borrow_mut()
                        .sync_from_report(report);
                    self.validation_map.queue_draw();
                    self.refresh_live_metrics();

                    let status_text = if let Some(error) = result.error.as_deref() {
                        if report.cancelled {
                            "Validation cancelled.".to_string()
                        } else {
                            error.to_string()
                        }
                    } else {
                        format!("Finished: {}", report_verdict(report))
                    };
                    self.set_status(&status_text);
                } else if let Some(error) = result.error.as_deref() {
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
            Ok(report) => build_worker_result_from_parts(target, Some(report), None),
            Err(error) => build_worker_result_from_parts(target, error.report, Some(error.message)),
        }
    }

    fn build_worker_result_from_parts(
        target: TargetInfo,
        report: Option<ValidationReport>,
        error: Option<String>,
    ) -> WorkerResult {
        let report_text = report
            .as_ref()
            .map(|report| format_report_text(&target, report))
            .or_else(|| {
                error
                    .as_ref()
                    .map(|error| format!("DriveCk\nTarget: {}\n\n{error}", target.path))
            });
        WorkerResult {
            target,
            report,
            report_text,
            error,
        }
    }

    fn show_save_report_dialog(state: &Rc<RefCell<AppState>>) {
        let (target, report, window) = {
            let state = state.borrow();
            match (state.last_target.clone(), state.last_report.clone()) {
                (Some(target), Some(report)) => (target, report, state.window.clone()),
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
    }

    fn build_summary_key(text: &str) -> Label {
        let label = Label::new(Some(text));
        label.add_css_class("summary-key");
        label.set_xalign(0.0);
        label
    }

    fn build_summary_value(text: &str) -> Label {
        let label = Label::new(Some(text));
        label.add_css_class("summary-value");
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_selectable(true);
        label.set_hexpand(true);
        label
    }

    fn append_summary_pair_row(
        grid: &Grid,
        row: i32,
        left_key: &str,
        left_value: &str,
        right_key: &str,
        right_value: &str,
    ) {
        let left_key = build_summary_key(left_key);
        let left_value = build_summary_value(left_value);
        let right_key = build_summary_key(right_key);
        let right_value = build_summary_value(right_value);
        grid.attach(&left_key, 0, row, 1, 1);
        grid.attach(&left_value, 1, row, 1, 1);
        grid.attach(&right_key, 2, row, 1, 1);
        grid.attach(&right_value, 3, row, 1, 1);
    }

    fn append_summary_full_row(grid: &Grid, row: i32, key: &str, value: &str) {
        let key = build_summary_key(key);
        let value = build_summary_value(value);
        grid.attach(&key, 0, row, 1, 1);
        grid.attach(&value, 1, row, 3, 1);
    }

    fn format_failure_summary(report: &ValidationReport) -> String {
        let mut parts = Vec::new();
        if report.read_error_count != 0 {
            parts.push(format!("read {}", report.read_error_count));
        }
        if report.write_error_count != 0 {
            parts.push(format!("write {}", report.write_error_count));
        }
        if report.mismatch_count != 0 {
            parts.push(format!("mismatch {}", report.mismatch_count));
        }
        if report.restore_error_count != 0 {
            parts.push(format!("restore {}", report.restore_error_count));
        }
        if report.cancelled {
            parts.push("cancelled".to_string());
        } else if !report.completed_all_samples {
            parts.push("incomplete".to_string());
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(" · ")
        }
    }

    fn report_issue_count(report: &ValidationReport) -> usize {
        report.read_error_count
            + report.write_error_count
            + report.mismatch_count
            + report.restore_error_count
    }

    fn report_verdict_class(report: &ValidationReport) -> &'static str {
        if report_issue_count(report) != 0 {
            "metric-danger"
        } else if report.cancelled || !report.completed_all_samples {
            "metric-neutral"
        } else {
            "metric-success"
        }
    }

    fn report_samples_class(report: &ValidationReport) -> &'static str {
        if report.completed_all_samples && !report.cancelled {
            "metric-success"
        } else {
            "metric-neutral"
        }
    }

    fn report_failure_chip(report: &ValidationReport) -> (String, &'static str) {
        let failure_count = report_issue_count(report);
        if failure_count != 0 {
            (format!("Failures {failure_count}"), "metric-danger")
        } else if report.cancelled {
            ("Cancelled".to_string(), "metric-neutral")
        } else if !report.completed_all_samples {
            ("Incomplete".to_string(), "metric-neutral")
        } else {
            ("No failures".to_string(), "metric-success")
        }
    }

    fn build_report_summary(
        target: Option<&TargetInfo>,
        report: Option<&ValidationReport>,
    ) -> GtkBox {
        let summary_box = GtkBox::new(Orientation::Vertical, 8);
        summary_box.add_css_class("panel");

        let summary_title = Label::new(Some("Summary"));
        summary_title.add_css_class("panel-title");
        summary_title.set_xalign(0.0);
        summary_box.append(&summary_title);

        let grid = Grid::new();
        grid.set_column_spacing(14);
        grid.set_row_spacing(10);
        grid.set_hexpand(true);

        if let Some(report) = report {
            let target_path = target
                .map(|target| target.path.clone())
                .unwrap_or_else(|| "-".to_string());
            let samples = format!(
                "Samples {}/{}",
                report.completed_samples,
                driveck_core::DRIVECK_SAMPLE_COUNT
            );
            let failure_detail = format_failure_summary(report);
            let emphasis = GtkBox::new(Orientation::Horizontal, 8);
            emphasis.set_halign(Align::Start);
            emphasis.append(&build_metric_chip(
                report_verdict(report),
                report_verdict_class(report),
            ));
            emphasis.append(&build_metric_chip(&samples, report_samples_class(report)));
            let (failure_chip_text, failure_chip_class) = report_failure_chip(report);
            emphasis.append(&build_metric_chip(&failure_chip_text, failure_chip_class));
            summary_box.append(&emphasis);

            append_summary_full_row(&grid, 0, "Target", &target_path);
            append_summary_pair_row(
                &grid,
                1,
                "Reported",
                &format_bytes(report.reported_size_bytes),
                "Validated",
                &format_bytes(report.validated_drive_size_bytes),
            );
            append_summary_pair_row(
                &grid,
                2,
                "Highest valid",
                &format_bytes(report.highest_valid_region_bytes),
                "Region",
                &format_bytes(report.region_size_bytes),
            );
            if failure_detail != "none" {
                append_summary_full_row(&grid, 3, "Failure detail", &failure_detail);
            }
        } else {
            let target_path = target
                .map(|target| target.path.clone())
                .unwrap_or_else(|| "-".to_string());
            let emphasis = GtkBox::new(Orientation::Horizontal, 8);
            emphasis.set_halign(Align::Start);
            emphasis.append(&build_metric_chip("Summary unavailable", "metric-neutral"));
            summary_box.append(&emphasis);
            append_summary_pair_row(
                &grid,
                0,
                "Status",
                "Summary unavailable",
                "Target",
                &target_path,
            );
        }

        summary_box.append(&grid);
        summary_box
    }

    fn show_report_dialog(state: &Rc<RefCell<AppState>>) {
        let (window, report_text, target, report, can_save) = {
            let state = state.borrow();
            match state.report_text.clone() {
                Some(report_text) => (
                    state.window.clone(),
                    report_text,
                    state.last_target.clone(),
                    state.last_report.clone(),
                    state.last_report.is_some() && state.last_target.is_some(),
                ),
                None => return,
            }
        };

        let dialog = Dialog::builder()
            .transient_for(&window)
            .modal(true)
            .title("Detailed report")
            .default_width(760)
            .default_height(620)
            .build();

        let content = dialog.content_area();
        content.set_spacing(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let summary = build_report_summary(target.as_ref(), report.as_ref());
        content.append(&summary);

        let raw_title = Label::new(Some("Raw report"));
        raw_title.add_css_class("panel-title");
        raw_title.set_xalign(0.0);
        content.append(&raw_title);

        let scroller = ScrolledWindow::new();
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        scroller.set_min_content_height(280);
        let text_view = TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_monospace(true);
        text_view.set_top_margin(10);
        text_view.set_bottom_margin(10);
        text_view.set_left_margin(10);
        text_view.set_right_margin(10);
        text_view.buffer().set_text(&report_text);
        scroller.set_child(Some(&text_view));
        content.append(&scroller);

        let footer = GtkBox::new(Orientation::Horizontal, 8);
        footer.set_hexpand(true);
        footer.set_halign(Align::End);
        footer.set_margin_top(6);
        footer.set_margin_bottom(2);

        let copy_button = Button::with_label("Copy");
        let close_button = Button::with_label("Close");
        footer.append(&copy_button);
        if can_save {
            let save_button = Button::with_label("Save report");
            let state_for_save = state.clone();
            save_button.connect_clicked(move |_| {
                show_save_report_dialog(&state_for_save);
            });
            footer.append(&save_button);
        }
        footer.append(&close_button);
        content.append(&footer);

        let state_for_copy = state.clone();
        let report_text_for_copy = report_text.clone();
        copy_button.connect_clicked(move |_| {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&report_text_for_copy);
                state_for_copy.borrow().set_status("Report copied.");
            }
        });

        let dialog_for_close = dialog.clone();
        close_button.connect_clicked(move |_| {
            dialog_for_close.close();
        });
        dialog.present();
    }

    fn needs_privileged_helper() -> bool {
        unsafe { libc::geteuid() != 0 }
    }

    fn spawn_local_validation_worker(
        target: TargetInfo,
        cancel_requested: Arc<AtomicBool>,
    ) -> StartedValidation {
        let (sender, receiver) = mpsc::channel::<WorkerMessage>();
        let worker = thread::spawn(move || {
            let mut progress = |update: ProgressUpdate| {
                let _ = sender.send(WorkerMessage::Progress {
                    phase: update.phase.to_string(),
                    current: update.current,
                    total: update.total,
                    sample_index: update.sample_index,
                    sample_status: update.sample_status,
                    final_update: update.final_update,
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
        });

        StartedValidation {
            receiver,
            worker,
            helper_cancel_pipe: None,
            helper_process: None,
            auth_pending: false,
        }
    }

    fn spawn_privileged_validation_worker(target: TargetInfo) -> Result<StartedValidation, String> {
        let executable = env::current_exe()
            .map_err(|error| format!("Failed to locate the DriveCk executable: {error}"))?;

        let mut command = Command::new("pkexec");
        command
            .arg("--disable-internal-agent")
            .arg(executable)
            .arg("--validate-helper")
            .arg(&target.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "pkexec is not installed. Install polkit/pkexec to validate from the GTK app."
                    .to_string()
            } else {
                format!("Failed to start GUI authentication with pkexec: {error}")
            }
        })?;

        let helper_cancel_pipe = child
            .stdin
            .take()
            .map(|stdin| Arc::new(Mutex::new(stdin)))
            .ok_or_else(|| {
                "Failed to open the privileged validation control channel.".to_string()
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture privileged validation output.".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Failed to capture privileged validation diagnostics.".to_string())?;
        let helper_process = Arc::new(Mutex::new(child));

        let (sender, receiver) = mpsc::channel::<WorkerMessage>();
        let worker = {
            let helper_process = helper_process.clone();
            let target = target.clone();
            thread::spawn(move || {
                run_privileged_validation_worker(target, helper_process, stdout, stderr, sender);
            })
        };

        Ok(StartedValidation {
            receiver,
            worker,
            helper_cancel_pipe: Some(helper_cancel_pipe),
            helper_process: Some(helper_process),
            auth_pending: true,
        })
    }

    fn run_privileged_validation_worker(
        target: TargetInfo,
        helper_process: Arc<Mutex<Child>>,
        stdout: ChildStdout,
        stderr: ChildStderr,
        sender: mpsc::Sender<WorkerMessage>,
    ) {
        let stderr_thread = thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            text
        });

        let mut finished = None;
        let mut parse_error = None;
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    parse_error = Some(format!(
                        "Failed to read privileged validation output: {error}"
                    ));
                    break;
                }
            };
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<HelperEvent>(&line) {
                Ok(HelperEvent::Started) => {
                    let _ = sender.send(WorkerMessage::Authenticated);
                }
                Ok(HelperEvent::Progress {
                    phase,
                    current,
                    total,
                    sample_index,
                    sample_status,
                    final_update,
                }) => {
                    let _ = sender.send(WorkerMessage::Progress {
                        phase,
                        current,
                        total,
                        sample_index,
                        sample_status,
                        final_update,
                    });
                }
                Ok(HelperEvent::Finished { report, error }) => {
                    finished = Some(build_worker_result_from_parts(
                        target.clone(),
                        report,
                        error,
                    ));
                }
                Err(error) => {
                    parse_error = Some(format!(
                        "Privileged validation returned unexpected output: {error}"
                    ));
                    break;
                }
            }
        }

        let exit_status = wait_for_helper_exit(&helper_process);
        let stderr_text = stderr_thread.join().unwrap_or_default();
        let worker_result = finished.unwrap_or_else(|| {
            build_worker_result_from_parts(
                target,
                None,
                Some(helper_process_error(exit_status, &stderr_text, parse_error)),
            )
        });
        let _ = sender.send(WorkerMessage::Finished(worker_result));
    }

    fn wait_for_helper_exit(helper_process: &Arc<Mutex<Child>>) -> ExitStatus {
        match helper_process.lock() {
            Ok(mut child) => child
                .wait()
                .unwrap_or_else(|_| ExitStatus::from_raw(1 << 8)),
            Err(_) => ExitStatus::from_raw(1 << 8),
        }
    }

    fn helper_process_error(
        exit_status: ExitStatus,
        stderr_text: &str,
        parse_error: Option<String>,
    ) -> String {
        if let Some(parse_error) = parse_error {
            return parse_error;
        }

        let stderr_text = stderr_text.trim();
        if !stderr_text.is_empty() {
            return stderr_text.to_string();
        }

        if exit_status.code().is_none() {
            return "Validation cancelled.".to_string();
        }

        match exit_status.code() {
            Some(126) => "Administrator authentication was cancelled.".to_string(),
            Some(127) => {
                "GUI authentication failed because no polkit agent handled the request.".to_string()
            }
            Some(code) => format!("Privileged validation exited with status code {code}."),
            None => "Validation cancelled.".to_string(),
        }
    }

    fn write_helper_event<W: io::Write>(
        writer: &mut BufWriter<W>,
        event: &HelperEvent,
    ) -> io::Result<()> {
        serde_json::to_writer(&mut *writer, event).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }

    pub fn parse_launch_mode(args: &[String]) -> Result<LaunchMode, String> {
        match args.get(1).map(String::as_str) {
            Some("--validate-helper") => {
                let device_path = args
                    .get(2)
                    .ok_or_else(|| "--validate-helper requires a device path.".to_string())?;
                if args.len() != 3 {
                    return Err(
                        "--validate-helper accepts exactly one whole-device path.".to_string()
                    );
                }
                Ok(LaunchMode::ValidateHelper {
                    device_path: device_path.clone(),
                })
            }
            _ => Ok(LaunchMode::App),
        }
    }

    pub fn run_validate_helper(device_path: &str) -> i32 {
        let stdout = io::stdout();
        let mut writer = BufWriter::new(stdout.lock());
        let target = match discover_target(device_path) {
            Ok(target) => target,
            Err(error) => {
                let _ = write_helper_event(
                    &mut writer,
                    &HelperEvent::Finished {
                        report: None,
                        error: Some(error.message),
                    },
                );
                return 2;
            }
        };

        if write_helper_event(&mut writer, &HelperEvent::Started).is_err() {
            return 2;
        }

        let cancel_requested = Arc::new(AtomicBool::new(false));
        {
            let cancel_requested = cancel_requested.clone();
            thread::spawn(move || {
                let stdin = io::stdin();
                let reader = BufReader::new(stdin.lock());
                for line in reader.lines() {
                    match line {
                        Ok(line) if line.trim().eq_ignore_ascii_case("cancel") => {
                            cancel_requested.store(true, Ordering::Relaxed);
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            });
        }

        let writer_failed = Arc::new(AtomicBool::new(false));
        let result = {
            let progress_failed = writer_failed.clone();
            let cancel_requested = cancel_requested.clone();
            let mut progress = |update: ProgressUpdate| {
                if progress_failed.load(Ordering::Relaxed) {
                    return;
                }

                let event = HelperEvent::Progress {
                    phase: update.phase.to_string(),
                    current: update.current,
                    total: update.total,
                    sample_index: update.sample_index,
                    sample_status: update.sample_status,
                    final_update: update.final_update,
                };
                if write_helper_event(&mut writer, &event).is_err() {
                    progress_failed.store(true, Ordering::Relaxed);
                }
            };
            let cancel = || {
                cancel_requested.load(Ordering::Relaxed) || writer_failed.load(Ordering::Relaxed)
            };
            validate_target_with_callbacks(
                &target,
                &ValidationOptions { seed: None },
                Some(&mut progress),
                Some(&cancel),
            )
        };

        let (report, error, exit_code) = match result {
            Ok(report) => (Some(report), None, 0),
            Err(error) => (error.report, Some(error.message), 1),
        };
        let _ = write_helper_event(&mut writer, &HelperEvent::Finished { report, error });
        exit_code
    }

    fn device_display_name(target: &TargetInfo) -> String {
        let name = format!("{} {}", target.vendor, target.model)
            .trim()
            .to_string();
        if !name.is_empty() {
            name
        } else if !target.name.is_empty() {
            target.name.clone()
        } else {
            target.path.clone()
        }
    }

    fn device_transport_text(target: &TargetInfo) -> &'static str {
        match (target.is_usb, target.is_removable) {
            (true, true) => "USB / Removable",
            (true, false) => "USB",
            (false, true) => "Removable",
            (false, false) => "Block",
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
            if target.is_mounted { " [mounted]" } else { "" }
        )
    }

    fn install_css() {
        let provider = CssProvider::new();
        provider.load_from_data(APP_CSS);
        if let Some(display) = gdk::Display::default() {
            style_context_add_provider_for_display(
                &display,
                &provider,
                STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    fn build_metric_chip(text: &str, class_name: &str) -> Label {
        let label = Label::new(Some(text));
        label.add_css_class("metric-chip");
        label.add_css_class(class_name);
        label
    }

    fn apply_chip_variant(label: &Label, class_name: &str) {
        label.remove_css_class("metric-neutral");
        label.remove_css_class("metric-success");
        label.remove_css_class("metric-danger");
        label.add_css_class(class_name);
    }

    fn draw_validation_map(
        context: &gtk::cairo::Context,
        width: i32,
        height: i32,
        grid_state: &ValidationGridState,
    ) {
        let width = width as f64;
        let height = height as f64;
        let cell_width = ((width - 2.0 * GRID_PADDING - GRID_GAP * (GRID_COLUMNS as f64 - 1.0))
            / GRID_COLUMNS as f64)
            .max(1.0);
        let cell_height = ((height - 2.0 * GRID_PADDING - GRID_GAP * (GRID_ROWS as f64 - 1.0))
            / GRID_ROWS as f64)
            .max(1.0);
        let cell = cell_width.min(cell_height);
        let grid_width = cell * GRID_COLUMNS as f64 + GRID_GAP * (GRID_COLUMNS as f64 - 1.0);
        let grid_height = cell * GRID_ROWS as f64 + GRID_GAP * (GRID_ROWS as f64 - 1.0);
        let origin_x = (width - grid_width) / 2.0;
        let origin_y = (height - grid_height) / 2.0;

        context.set_source_rgb(0.965, 0.972, 0.984);
        let _ = context.paint();

        for row in 0..GRID_ROWS {
            for column in 0..GRID_COLUMNS {
                let index = row * GRID_COLUMNS + column;
                let x = origin_x + column as f64 * (cell + GRID_GAP);
                let y = origin_y + row as f64 * (cell + GRID_GAP);
                let (red, green, blue) = match grid_state.sample_status.get(index).copied() {
                    Some(SampleStatus::Untested) => (0.84, 0.88, 0.93),
                    Some(SampleStatus::Ok) => (0.12, 0.67, 0.39),
                    Some(
                        SampleStatus::ReadError
                        | SampleStatus::WriteError
                        | SampleStatus::VerifyMismatch
                        | SampleStatus::RestoreError,
                    ) => (0.86, 0.28, 0.29),
                    None => (0.92, 0.94, 0.97),
                };

                context.set_source_rgb(red, green, blue);
                context.rectangle(x, y, cell, cell);
                let _ = context.fill();

                if index < grid_state.sample_status.len() && grid_state.last_sample == Some(index) {
                    context.set_source_rgb(1.0, 1.0, 1.0);
                    context.set_line_width(2.0);
                    context.rectangle(
                        x + 1.0,
                        y + 1.0,
                        (cell - 2.0).max(1.0),
                        (cell - 2.0).max(1.0),
                    );
                    let _ = context.stroke();
                }
            }
        }
    }

    fn build_ui(application: &Application) -> Rc<RefCell<AppState>> {
        install_css();

        let window = ApplicationWindow::builder()
            .application(application)
            .title("DriveCk")
            .default_width(620)
            .default_height(580)
            .build();
        window.set_resizable(true);

        let root = GtkBox::new(Orientation::Vertical, 8);
        root.add_css_class("window-root");
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let device_panel = GtkBox::new(Orientation::Vertical, 8);
        device_panel.add_css_class("panel");
        device_panel.set_hexpand(true);

        let device_row = GtkBox::new(Orientation::Horizontal, 6);
        let device_model = StringList::new(&[]);
        let device_dropdown = DropDown::new(Some(device_model.clone()), None::<gtk::Expression>);
        device_dropdown.set_hexpand(true);
        let refresh_button = Button::with_label("Refresh");
        let action_button = Button::with_label("Validate");
        action_button.add_css_class("suggested-action");
        device_row.append(&device_dropdown);
        device_row.append(&refresh_button);
        device_row.append(&action_button);
        device_panel.append(&device_row);

        let device_title_label = Label::new(Some("No device available"));
        device_title_label.add_css_class("device-title");
        device_title_label.set_xalign(0.0);
        let device_meta_row = GtkBox::new(Orientation::Horizontal, 6);
        let device_size_label = build_metric_chip("No device", "metric-neutral");
        let device_transport_label = build_metric_chip("USB / Removable", "metric-neutral");
        let device_state_label = build_metric_chip("Waiting", "metric-neutral");
        device_meta_row.append(&device_size_label);
        device_meta_row.append(&device_transport_label);
        device_meta_row.append(&device_state_label);
        let status_label = build_metric_chip("Select a device to begin.", "metric-neutral");
        status_label.set_xalign(0.0);
        device_panel.append(&status_label);
        device_panel.append(&device_title_label);
        device_panel.append(&device_meta_row);
        root.append(&device_panel);

        let map_panel = GtkBox::new(Orientation::Vertical, 8);
        map_panel.add_css_class("panel");
        map_panel.set_hexpand(true);
        let map_top = GtkBox::new(Orientation::Horizontal, 8);
        let map_title = Label::new(Some("Validation map"));
        map_title.add_css_class("panel-title");
        map_title.set_xalign(0.0);
        map_title.set_hexpand(true);
        let report_button = Button::with_label("Report");
        report_button.set_sensitive(false);
        map_top.append(&map_title);
        map_top.append(&report_button);
        map_panel.append(&map_top);

        let validation_grid_state = Rc::new(RefCell::new(ValidationGridState::default()));
        let map_frame = AspectFrame::new(0.5, 0.5, GRID_COLUMNS as f32 / GRID_ROWS as f32, false);
        map_frame.set_hexpand(true);
        map_frame.set_halign(Align::Fill);
        map_frame.set_height_request(GRID_HEIGHT);
        let validation_map = DrawingArea::new();
        validation_map.set_hexpand(true);
        validation_map.set_vexpand(true);
        {
            let validation_grid_state = validation_grid_state.clone();
            validation_map.set_draw_func(move |_, context, width, height| {
                if let Ok(grid_state) = validation_grid_state.try_borrow() {
                    draw_validation_map(context, width, height, &grid_state);
                }
            });
        }
        map_frame.set_child(Some(&validation_map));
        map_panel.append(&map_frame);
        let map_footer = GtkBox::new(Orientation::Vertical, 6);
        map_footer.set_hexpand(true);
        let metrics_row = GtkBox::new(Orientation::Horizontal, 8);
        metrics_row.set_halign(Align::Center);
        let processed_label = build_metric_chip(
            &format!("Done 0/{}", driveck_core::DRIVECK_SAMPLE_COUNT),
            "metric-neutral",
        );
        let ok_label = build_metric_chip("OK 0", "metric-success");
        let fail_label = build_metric_chip("Fail 0", "metric-neutral");
        metrics_row.append(&processed_label);
        metrics_row.append(&ok_label);
        metrics_row.append(&fail_label);
        map_footer.append(&metrics_row);
        map_panel.append(&map_footer);
        root.append(&map_panel);

        let version_footer = GtkBox::new(Orientation::Horizontal, 0);
        version_footer.set_hexpand(true);
        version_footer.set_halign(Align::Center);
        version_footer.set_margin_top(2);
        let version_link = LinkButton::builder()
            .uri(PROJECT_URL)
            .label(format!("DriveCk {APP_VERSION}"))
            .build();
        version_link.add_css_class("version-link");
        version_link.set_has_frame(false);
        version_footer.append(&version_link);
        root.append(&version_footer);

        window.set_child(Some(&root));

        let state = Rc::new(RefCell::new(AppState {
            window,
            device_dropdown,
            device_title_label,
            device_size_label,
            device_transport_label,
            device_state_label,
            processed_label,
            ok_label,
            fail_label,
            refresh_button,
            action_button,
            report_button,
            status_label,
            validation_map,
            validation_grid_state,
            device_model,
            device_targets: Vec::new(),
            report_text: None,
            last_target: None,
            last_report: None,
            worker: None,
            receiver: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            helper_cancel_pipe: None,
            helper_process: None,
            helper_auth_pending: false,
            confirmation_pending: false,
            stop_requested: false,
            closing_requested: false,
        }));

        {
            let state = state.clone();
            let button = state.borrow().refresh_button.clone();
            button.connect_clicked(move |_| state.borrow_mut().refresh_device_list());
        }
        {
            let state = state.clone();
            let button = state.borrow().action_button.clone();
            button.connect_clicked(move |_| {
                if state.borrow().is_busy() {
                    let mut state = state.borrow_mut();
                    if !state.stop_requested {
                        state.stop_requested = true;
                        state.request_stop();
                        state.set_status("Stopping...");
                        state.update_actions();
                    }
                    return;
                }

                let target = match state.borrow().prepare_selected_target() {
                    Ok(target) => target,
                    Err(error) => {
                        state.borrow().show_message("Cannot start validation.", &error);
                        return;
                    }
                };

                {
                    let mut state = state.borrow_mut();
                    state.confirmation_pending = true;
                    state.update_actions();
                }

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
                    dialog.close();
                    if response == ResponseType::Accept {
                        let state_for_start = state_for_response.clone();
                        let target = target.clone();
                        glib::idle_add_local_once(move || {
                            state_for_start.borrow_mut().start_validation(target);
                        });
                    } else if let Ok(mut state) = state_for_response.try_borrow_mut() {
                        state.confirmation_pending = false;
                        state.update_actions();
                    }
                });
                dialog.present();
            });
        }
        {
            let state = state.clone();
            let button = state.borrow().report_button.clone();
            button.connect_clicked(move |_| {
                show_report_dialog(&state);
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
                state.stop_requested = true;
                state.request_stop();
                state.set_status("Stopping before exit...");
                state.update_actions();
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

        state.borrow().refresh_live_metrics();
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

    #[cfg(test)]
    mod tests {
        use super::{parse_launch_mode, LaunchMode};

        #[test]
        fn launch_mode_defaults_to_app() {
            let args = vec!["driveck".to_string()];
            assert!(matches!(
                parse_launch_mode(&args).expect("mode should parse"),
                LaunchMode::App
            ));
        }

        #[test]
        fn launch_mode_parses_helper_path() {
            let args = vec![
                "driveck".to_string(),
                "--validate-helper".to_string(),
                "/dev/sdb".to_string(),
            ];
            match parse_launch_mode(&args).expect("mode should parse") {
                LaunchMode::ValidateHelper { device_path } => assert_eq!(device_path, "/dev/sdb"),
                LaunchMode::App => panic!("helper mode should be selected"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    match app::parse_launch_mode(&args) {
        Ok(app::LaunchMode::App) => app::run(),
        Ok(app::LaunchMode::ValidateHelper { device_path }) => {
            std::process::exit(app::run_validate_helper(&device_path));
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
