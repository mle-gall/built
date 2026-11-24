use crate::{absolute, python};
use pyo3::prelude::*;
use std::path::{Path, PathBuf};

struct PdfExport {
    input: PathBuf,
    output: PathBuf,
}

fn parse_pdf(args: &[String]) -> Result<Option<PdfExport>, String> {
    let flags = [
        "-i",
        "--input-file",
        "-o",
        "--output-file",
        "-p",
        "--create-pdf",
    ];
    if !args
        .iter()
        .any(|arg| flags.contains(&arg.split('=').next().unwrap()))
    {
        return Ok(None);
    }
    let mut input = None;
    let mut output = None;
    let mut pdf = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (flag, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        match flag {
            "-p" | "--create-pdf" if value.is_none() => pdf = true,
            "-i" | "--input-file" | "-o" | "--output-file" => {
                let path = value
                    .or_else(|| args.next().map(String::as_str))
                    .filter(|p| !p.is_empty() && !p.starts_with('-'))
                    .ok_or_else(|| format!("{flag} needs a file"))?;
                let dest = if flag == "-i" || flag == "--input-file" {
                    &mut input
                } else {
                    &mut output
                };
                *dest = Some(absolute(Path::new(path)));
            }
            _ => return Err(format!("unexpected export option {arg}")),
        }
    }
    let input = input.ok_or("--input-file is required")?;
    if !pdf {
        return Err("--create-pdf is required".into());
    }
    Ok(Some(PdfExport {
        output: output.unwrap_or_else(|| input.with_extension("pdf")),
        input,
    }))
}

fn export(job: &PdfExport) -> PyResult<i32> {
    let layout = python::Layout::find(None, None);
    python::start(&layout)?;
    let host = python::Host::new((0, 0, ""), 1280., 800., 1., None, true)?;
    Python::attach(|py| {
        let opened = host.call("open", (&job.input,));
        if !opened.is_some_and(|v| v.bind(py).is_truthy().unwrap_or(false)) {
            eprintln!("built: could not open {}", job.input.display());
            return Ok(1);
        }
        let window = host
            .call("__getattribute__", ("window",))
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("missing document window"))?;
        let sheets = window
            .bind(py)
            .getattr("viewport")?
            .getattr("scene")?
            .getattr("compositions")?;
        if sheets.len()? == 0 {
            eprintln!("built: {} has no sheets", job.input.display());
            return Ok(1);
        }
        if let Some(dir) = job.output.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let composer = py
            .import("views.wgpu_sheets")?
            .call_method1("composer_for", (window,))?;
        let troubled = composer.call_method1("export_all_pdf", (&job.output, sheets))?;
        if !job.output.is_file() {
            return Ok(1);
        }
        println!("Exported sheets to {}", job.output.display());
        Ok(if troubled.is_truthy()? { 3 } else { 0 })
    })
}

pub fn register_launcher() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--mcp"] {
        let layout = python::Layout::find(None, None);
        let result = python::start(&layout).and_then(|_| {
            Python::attach(|py| {
                py.import("scripts.ingetrazo_mcp")?.call_method0("main")?;
                Ok(())
            })
        });
        if let Err(error) = result {
            eprintln!("built: MCP server failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    match parse_pdf(&args) {
        Ok(None) => (),
        Ok(Some(job)) if !job.input.is_file() => {
            eprintln!("built: no such file: {}", job.input.display());
            std::process::exit(2);
        }
        Ok(Some(job)) => std::process::exit(export(&job).unwrap_or_else(|error| {
            eprintln!("built: PDF export failed: {error}");
            1
        })),
        Err(error) => {
            eprintln!("built: {error}");
            std::process::exit(2);
        }
    }
}

pub fn self_check(layout: &python::Layout) -> i32 {
    let result = Python::attach(|py| -> PyResult<()> {
        for rel in [
            "i18n/es.json",
            "resources/textures/library.json",
            "resources/components/components.json",
            "resources/icons/ingetrazo_256.png",
            "scripts/ingetrazo_mcp.py",
        ] {
            if !layout.root.join(rel).is_file() {
                return Err(pyo3::exceptions::PyFileNotFoundError::new_err(rel));
            }
        }
        py.import("core.ai_recipes")?
            .getattr("reference")?
            .call1(("llamada",))?;
        let converter = py
            .import("formats.dwg_bridge")?
            .call_method0("find_dwg2dxf")?;
        if converter.is_none() {
            println!("DWG converter: MISSING");
            let frozen = py
                .import("core.paths")?
                .call_method0("is_frozen")?
                .is_truthy()?;
            if frozen && !cfg!(target_os = "macos") {
                return Err(pyo3::exceptions::PyFileNotFoundError::new_err("dwg2dxf"));
            }
        } else {
            println!("DWG converter: {}", converter.str()?);
        }
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("built: {error}");
        return 1;
    }
    0
}

pub fn exit_now(status: i32) -> ! {
    Python::attach(|py| {
        let flush = || -> PyResult<()> {
            py.import("core.settings")?
                .call_method0("settings")?
                .call_method0("sync")?;
            py.import("logging")?.call_method0("shutdown")?;
            let sys = py.import("sys")?;
            for name in ["stdout", "stderr"] {
                let stream = sys.getattr(name)?;
                if !stream.is_none() {
                    stream.call_method0("flush")?;
                }
            }
            Ok(())
        };
        if let Err(error) = flush() {
            eprintln!("built: flushing settings and logs: {error}");
        }
    });
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        let _ = TerminateProcess(GetCurrentProcess(), status as u32);
    }
    std::process::exit(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<PdfExport>, String> {
        parse_pdf(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn ordinary_launches_pass_through() {
        for args in [&[][..], &["Casa.igz"], &["--new-instance"]] {
            assert!(parse(args).unwrap().is_none());
        }
    }

    #[test]
    fn output_defaults_to_the_input_stem() {
        let job = parse(&["-i", "Casa.igz", "-p"]).unwrap().unwrap();
        assert_eq!(job.input, absolute(Path::new("Casa.igz")));
        assert_eq!(job.output, absolute(Path::new("Casa.pdf")));
        let job = parse(&[
            "--input-file=Casa.igz",
            "--create-pdf",
            "--output-file",
            "out/plan.pdf",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(job.output, absolute(Path::new("out/plan.pdf")));
    }

    #[test]
    fn missing_input_or_task_is_an_error() {
        for args in [&["-p"][..], &["-i", "Casa.igz"]] {
            assert!(parse(args).is_err());
        }
    }
}
