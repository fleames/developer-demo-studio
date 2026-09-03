use std::sync::{Arc, atomic::AtomicBool};

use analysis::{AnalysisOptions, make_beautiful};
use project_model::Project;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: export_project <project.ddp>");
    let mut project = Project::open(root).expect("open project");
    let events = project.read_events().expect("read events");
    make_beautiful(&mut project.scene, &events, AnalysisOptions::default());
    project.save().expect("save analyzed scene");
    let destination = project.root.join("exports/debug-github.gif");
    let result = export::render_gif(
        &project,
        &destination,
        export::GifPreset::GITHUB,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            if ((progress * 100.0) as u32).is_multiple_of(10) {
                eprint!("\rRendering: {:>3}%", (progress * 100.0) as u32);
            }
        },
    )
    .expect("render GIF");
    println!(
        "\nExported {} frames ({} bytes) to {}",
        result.frames,
        result.bytes,
        result.path.display()
    );
}
