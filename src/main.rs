use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "chip", about = "Create, inspect and render chiptune scores.")]
struct Cli {
    #[arg(long, global = true, hide = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Describe,
    New {
        score: PathBuf,
        #[arg(long, default_value = "chip4")]
        profile: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "lanterns")]
        template: String,
    },
    Inspect {
        score: PathBuf,
    },
    Validate {
        score: PathBuf,
    },
    Patch {
        score: PathBuf,
        #[arg(long)]
        ops: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long = "expected-sha256")]
        expected: String,
    },
    Render {
        score: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 32000)]
        sample_rate: u32,
        #[arg(long, default_value_t = 0.)]
        from_bar: f64,
        #[arg(long)]
        bars: Option<f64>,
        #[arg(long)]
        no_stems: bool,
        #[arg(long)]
        solo: Option<String>,
    },
    Compare {
        manifests: Vec<PathBuf>,
    },
    Transcribe {
        wav: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 120.)]
        bpm: f64,
        #[arg(long, default_value_t = 0.25)]
        quantize: f64,
        #[arg(long, default_value = "chip4")]
        profile: String,
        #[arg(long)]
        melody_only: bool,
        #[arg(long)]
        title: Option<String>,
    },
    Studio {
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    Mcp {
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

fn data_root(value: Option<PathBuf>) -> Result<PathBuf> {
    let path = value
        .or_else(|| std::env::var_os("CHIPTUNE_DATA_DIR").map(PathBuf::from))
        .unwrap_or(std::env::current_dir()?.join("data"));
    std::fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}

fn run() -> Result<Option<Value>> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e)
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            e.print()?;
            return Ok(None);
        }
        Err(e) => return Err(anyhow!(e.to_string())),
    };
    use Command::*;
    Ok(Some(match cli.command {
        Describe => chiptune_studio::engine::describe(),
        New {
            score,
            profile,
            title,
            template,
        } => chiptune_studio::engine::create(&score, &profile, &title, &template)?,
        Inspect { score } => chiptune_studio::engine::inspect(&score)?,
        Validate { score } => {
            chiptune_studio::score::validate(&chiptune_studio::score::load(&score)?)?
        }
        Patch {
            score,
            ops,
            out,
            expected,
        } => {
            let operations: Value = serde_json::from_slice(&std::fs::read(ops)?)?;
            chiptune_studio::engine::patch(&score, &operations, &out, &expected)?
        }
        Render {
            score,
            out,
            sample_rate,
            from_bar,
            bars,
            no_stems,
            solo,
        } => {
            let solo = solo
                .map(|s| {
                    s.split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            chiptune_studio::engine::render(
                &score,
                &out,
                sample_rate,
                from_bar,
                bars,
                !no_stems,
                &solo,
            )?
        }
        Compare { manifests } => chiptune_studio::engine::compare(&manifests)?,
        Transcribe {
            wav,
            out,
            bpm,
            quantize,
            profile,
            melody_only,
            title,
        } => chiptune_studio::transcribe::import_take(
            &wav,
            &out,
            bpm,
            quantize,
            &profile,
            !melody_only,
            title.as_deref().unwrap_or("Take"),
        )?,
        Studio { root, port } => {
            chiptune_studio::server::studio(&data_root(root)?, port)?;
            json!({"ok": true})
        }
        Mcp { root } => {
            chiptune_studio::server::mcp(&data_root(root)?)?;
            return Ok(None);
        }
    }))
}

fn main() {
    match run() {
        Ok(Some(value)) => println!(
            "{}",
            serde_json::to_string(&value).unwrap_or_else(|_| "{}".into())
        ),
        Ok(None) => {}
        Err(error) => {
            println!("{}", json!({"ok": false, "error": format!("{:#}", error)}));
            std::process::exit(2);
        }
    }
}
