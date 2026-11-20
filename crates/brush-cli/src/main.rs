//! `brush`: the engine on the command line. Every command prints JSON to stdout so scripts, desktop tools and AI
//! agents can drive it.

use brush_engine::io::{self, Format};
use brush_engine::render::{render, Camera};
use brush_engine::{Actor, ActorKind, Document, Journal, Mesh, Op};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "brush",
    version,
    about = "Inspect, convert, edit, render and replay PLY meshes with typed operations."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print counts, bounds, attributes and the content hash of a mesh.
    Info { file: PathBuf },
    /// Convert between PLY, OBJ and GLB (by extension).
    Convert { input: PathBuf, output: PathBuf },
    /// Apply ops to a mesh. Ops are a JSON array, one JSON op, or JSON Lines; `-` reads stdin.
    Apply {
        input: PathBuf,
        #[arg(long)]
        ops: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Also write the journal (JSON Lines) here.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Who is making the change, recorded in the journal.
        #[arg(long, default_value = "")]
        actor: String,
        /// The actor is an AI model.
        #[arg(long)]
        model: bool,
    },
    /// Replay a journal onto a mesh, checking every step's hash.
    Replay {
        input: PathBuf,
        journal: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Render a view to PNG with the engine's software renderer.
    Render {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Camera position, `x,y,z`. Default: in front of the mesh, framing it.
        #[arg(long, value_parser = vec3)]
        eye: Option<[f64; 3]>,
        /// Point looked at, `x,y,z`. Default: the centre of the mesh.
        #[arg(long, value_parser = vec3)]
        target: Option<[f64; 3]>,
        #[arg(long, default_value_t = 60.0)]
        fov: f64,
        #[arg(long, default_value_t = 800)]
        width: u32,
        #[arg(long, default_value_t = 600)]
        height: u32,
    },
    /// Print a JSON Schema: `ops` (the operation contract) or `journal`.
    Schema {
        #[arg(default_value = "ops")]
        which: String,
    },
}

fn vec3(s: &str) -> Result<[f64; 3], String> {
    let v: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    v.try_into().map_err(|_| "expected x,y,z".to_string())
}

type Res<T> = Result<T, String>;

fn format_of(path: &Path) -> Res<Format> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_extension)
        .ok_or_else(|| {
            format!(
                "{}: unknown extension (use .ply, .obj or .glb)",
                path.display()
            )
        })
}

fn load(path: &Path) -> Res<Mesh> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let format = Format::sniff(&bytes).map_or_else(|| format_of(path), Ok)?;
    let (mesh, report) =
        io::read(&bytes, format).map_err(|e| format!("{}: {e}", path.display()))?;
    if report != io::ReadReport::default() {
        eprintln!("{}: {report:?}", path.display());
    }
    Ok(mesh)
}

fn save(mesh: &Mesh, path: &Path) -> Res<()> {
    std::fs::write(path, io::write(mesh, format_of(path)?))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn read_text(path: &Path) -> Res<String> {
    if path == Path::new("-") {
        std::io::read_to_string(std::io::stdin()).map_err(|e| e.to_string())
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A JSON array of ops, a single op, or one op per line.
fn parse_ops(text: &str) -> Res<Vec<Op>> {
    let t = text.trim();
    if t.starts_with('[') {
        return serde_json::from_str(t).map_err(|e| format!("ops: {e}"));
    }
    if let Ok(one) = serde_json::from_str::<Op>(t) {
        return Ok(vec![one]);
    }
    t.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("ops line {}: {e}", i + 1)))
        .collect()
}

fn info(mesh: &Mesh) -> Value {
    let bounds = mesh.bounds().map(|(lo, hi)| json!({"min": lo, "max": hi}));
    json!({
        "vertices": mesh.vertex_count(),
        "faces": mesh.face_count(),
        "bounds": bounds,
        "attributes": mesh.attributes.iter().map(|a| json!({"name": a.name, "kind": a.kind})).collect::<Vec<_>>(),
        "hash": mesh.content_hash(),
    })
}

fn run(cli: Cli) -> Res<Value> {
    match cli.cmd {
        Cmd::Info { file } => Ok(info(&load(&file)?)),
        Cmd::Convert { input, output } => {
            let m = load(&input)?;
            save(&m, &output)?;
            Ok(info(&m))
        }
        Cmd::Apply {
            input,
            ops,
            output,
            journal,
            actor,
            model,
        } => {
            let mut doc = Document::new(load(&input)?);
            let ops = parse_ops(&read_text(&ops)?)?;
            let actor = Actor {
                id: actor,
                kind: model.then_some(ActorKind::Model),
            };
            let applied = doc
                .apply_all(ops, actor)
                .map_err(|(i, e)| format!("op {}: {e}", i + 1))?;
            save(doc.mesh(), &output)?;
            if let Some(j) = journal {
                std::fs::write(&j, doc.journal().to_jsonl())
                    .map_err(|e| format!("{}: {e}", j.display()))?;
            }
            Ok(json!({"applied": applied, "mesh": info(doc.mesh())}))
        }
        Cmd::Replay {
            input,
            journal,
            output,
        } => {
            let j =
                Journal::from_jsonl(&read_text(&journal)?).map_err(|e| format!("journal: {e}"))?;
            let doc = Document::replay(load(&input)?, &j).map_err(|e| e.to_string())?;
            if let Some(o) = output {
                save(doc.mesh(), &o)?;
            }
            Ok(json!({"entries": j.entries.len(), "mesh": info(doc.mesh())}))
        }
        Cmd::Render {
            input,
            output,
            eye,
            target,
            fov,
            width,
            height,
        } => {
            let m = load(&input)?;
            let (lo, hi) = m.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
            let centre = [0, 1, 2].map(|i| (lo[i] as f64 + hi[i] as f64) / 2.0);
            let radius = (0..3)
                .map(|i| (hi[i] - lo[i]) as f64)
                .fold(0.0, f64::max)
                .max(0.1);
            let target = target.unwrap_or(centre);
            let eye = eye.unwrap_or([
                centre[0] + radius * 0.6,
                centre[1] + radius * 0.5,
                centre[2] + radius * 1.4,
            ]);
            let cam = Camera {
                eye,
                target,
                fov_deg: fov,
            };
            let px = render(&m, width, height, &cam);
            let file =
                std::fs::File::create(&output).map_err(|e| format!("{}: {e}", output.display()))?;
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()
                .and_then(|mut w| w.write_image_data(&px))
                .map_err(|e| e.to_string())?;
            Ok(json!({"camera": cam, "width": width, "height": height}))
        }
        Cmd::Schema { which } => match which.as_str() {
            "ops" => Ok(brush_engine::mesh::ops_schema()),
            "journal" => Ok(brush_engine::journal_schema()),
            other => Err(format!("unknown schema {other}; use ops or journal")),
        },
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).expect("json"));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("brush: {e}");
            ExitCode::FAILURE
        }
    }
}
