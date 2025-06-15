use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use serde_json::Value;
use chrono::{DateTime, Utc};

mod media;

#[derive(Default)]
struct MetadataApp {
    input_dir: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    input_dir_text: String,
    output_dir_text: String,
    is_processing: bool,
    progress: f32,
    status_messages: Vec<String>,
    processed_count: usize,
    error_count: usize,
    total_files: usize,
    receiver: Option<mpsc::Receiver<ProcessMessage>>,
}

#[derive(Debug)]
enum ProcessMessage {
    Progress(f32),
    Status(String),
    FileProcessed(String, bool),
    Completed(usize, usize),
    Error(String),
}

impl eframe::App for MetadataApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut should_clear_receiver = false;

        if let Some(receiver) = &self.receiver {
            while let Ok(msg) = receiver.try_recv() {
                match msg {
                    ProcessMessage::Progress(p) => self.progress = p,
                    ProcessMessage::Status(s) => {
                        self.status_messages.push(s);
                        if self.status_messages.len() > 100 {
                            self.status_messages.remove(0);
                        }
                    }
                    ProcessMessage::FileProcessed(file, success) => {
                        if success {
                            self.processed_count += 1;
                            self.status_messages.push(format!("✅ {}", file));
                        } else {
                            self.error_count += 1;
                            self.status_messages.push(format!("❌ {}", file));
                        }
                        if self.status_messages.len() > 100 {
                            self.status_messages.remove(0);
                        }
                    }
                    ProcessMessage::Completed(processed, errors) => {
                        self.is_processing = false;
                        self.processed_count = processed;
                        self.error_count = errors;
                        self.status_messages.push(format!(
                            "🎉 Processing complete! {} files processed, {} errors",
                            processed, errors
                        ));
                        should_clear_receiver = true;
                    }
                    ProcessMessage::Error(e) => {
                        self.is_processing = false;
                        self.status_messages.push(format!("💥 Fatal error: {}", e));
                        should_clear_receiver = true;
                    }
                }
            }
        }

        if should_clear_receiver {
            self.receiver = None;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("📷 Metadata Fix 🎬");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("📁 Input Directory:");
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.input_dir_text = path.display().to_string();
                        self.input_dir = Some(path);
                    }
                }
            });
            ui.text_edit_singleline(&mut self.input_dir_text);
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label("📤 Output Directory:");
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_dir_text = path.display().to_string();
                        self.output_dir = Some(path);
                    }
                }
            });
            ui.text_edit_singleline(&mut self.output_dir_text);
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                let can_process = self.input_dir.is_some() 
                && self.output_dir.is_some() 
                && !self.is_processing;

                if ui.add_enabled(can_process, egui::Button::new("Process Media"))
                    .clicked() {
                    self.start_processing();
                }

                if self.is_processing {
                    ui.spinner();
                    ui.label("Processing...");
                }
            });

            ui.add_space(20.0);

            if self.is_processing || self.progress > 0.0 {
                ui.label(format!("Progress: {:.1}%", self.progress * 100.0));
                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                ui.add_space(10.0);

                ui.label(format!(
                    "Processed: {} | Errors: {} | Total: {}",
                    self.processed_count, self.error_count, self.total_files
                ));
                ui.add_space(10.0);
            }

            if !self.status_messages.is_empty() {
                ui.label("📋 Status Log:");
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for message in &self.status_messages {
                            ui.label(message);
                        }
                    });
            }
        });

        if self.is_processing {
            ctx.request_repaint();
        }
    }
}

impl MetadataApp {
    fn start_processing(&mut self) {
        let input_dir = self.input_dir.clone().unwrap();
        let output_dir = self.output_dir.clone().unwrap();

        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.is_processing = true;
        self.progress = 0.0;
        self.processed_count = 0;
        self.error_count = 0;
        self.status_messages.clear();

        thread::spawn(move || {
            process_photos(input_dir, output_dir, sender);
        });
    }
}

fn process_photos(
    input_dir: PathBuf,
    output_dir: PathBuf,
    sender: mpsc::Sender<ProcessMessage>,
) {
    let _ = sender.send(ProcessMessage::Status("🔍 Scanning directories...".to_string()));

    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        let _ = sender.send(ProcessMessage::Error(format!("Could not create output directory: {}", e)));
        return;
    }

    let mut json_files = Vec::new();
    let mut dirs_to_check = vec![input_dir.clone()];

    while let Some(dir) = dirs_to_check.pop() {
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        dirs_to_check.push(path);
                    } else if path.extension().map_or(false, |ext| ext == "json") {
                        json_files.push(path);
                    }
                }
            }
            Err(_) => continue,
        }
    }

    let total_files = json_files.len();
    let _ = sender.send(ProcessMessage::Status(format!("📊 Found {} JSON files to process", total_files)));

    let mut processed_count = 0;
    let mut error_count = 0;

    for (index, json_file) in json_files.iter().enumerate() {
        let progress = index as f32 / total_files as f32;
        let _ = sender.send(ProcessMessage::Progress(progress));

        match process_single_file(json_file, &input_dir, &output_dir) {
            Ok(image_name) => {
                processed_count += 1;
                let _ = sender.send(ProcessMessage::FileProcessed(image_name, true));
            }
            Err(e) => {
                error_count += 1;
                let _ = sender.send(ProcessMessage::FileProcessed(
                    format!("{}: {}", json_file.file_name().unwrap_or_default().to_string_lossy(), e),
                    false
                ));
            }
        }
    }

    let _ = sender.send(ProcessMessage::Progress(1.0));
    let _ = sender.send(ProcessMessage::Completed(processed_count, error_count));
}

fn process_single_file(
    json_file: &PathBuf,
    input_dir: &PathBuf,
    output_dir: &PathBuf,
) -> Result<String, String> {
    let json_string = std::fs::read_to_string(json_file)
        .map_err(|e| format!("Error reading JSON: {}", e))?;

    let json_data: Value = serde_json::from_str(&json_string)
        .map_err(|e| format!("Error parsing JSON: {}", e))?;

    let media_name = json_data["title"].as_str()
        .ok_or("No title found in JSON")?;

    let latitude = json_data["geoData"]["latitude"].as_f64()
        .ok_or("No latitude found in JSON")?;

    let longitude = json_data["geoData"]["longitude"].as_f64()
        .ok_or("No longitude found in JSON")?;

    let altitude = json_data["geoData"]["altitude"].as_f64().unwrap_or(0.0);

    let timestamp_str = json_data["photoTakenTime"]["timestamp"].as_str()
        .ok_or("No timestamp found in JSON")?;

    let timestamp: i64 = timestamp_str.parse()
        .map_err(|_| "Invalid timestamp format")?;

    let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
        .ok_or("Invalid timestamp value")?;

    let image_path = json_file.parent().unwrap().join(media_name);
    if !image_path.exists() {
        return Err("Image file not found".to_string());
    }

    let relative_path = image_path.strip_prefix(input_dir)
        .map_err(|_| "Could not determine relative path")?;
    let output_path = output_dir.join(relative_path);

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Error creating output directory: {}", e))?;
    }

    let image_path_str = image_path.to_string_lossy();
    let output_path_str = output_path.to_string_lossy();

    if media_name.to_lowercase().ends_with(".jpg") || media_name.to_lowercase().ends_with(".jpeg") {
        media::update_jpeg_metadata(&image_path_str, Some(&output_path_str), latitude, longitude, altitude, datetime)
            .map_err(|e| format!("JPEG processing error: {}", e))?;
    } else if media_name.to_lowercase().ends_with(".png") {
        media::update_png_metadata(&image_path_str, Some(&output_path_str), latitude, longitude, altitude, datetime)
            .map_err(|e| format!("PNG processing error: {}", e))?;
    } else {
        return Err("Unsupported file format".to_string());
    }

    Ok(media_name.to_string())
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Metadata Fix",
        options,
        Box::new(|_cc| Ok(Box::<MetadataApp>::default())),
    )
}
