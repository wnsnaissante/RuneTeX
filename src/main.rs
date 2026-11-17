#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod compiler;
mod font_picker;
mod icons;
mod language;
mod lsp;
mod paths;
mod preferences;
mod preview;
mod preview_cache;
mod state;
mod ui;

use rune::{App, AppSpec};
use state::State;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(all(windows, not(debug_assertions)))]
    if args.len() > 1 {
        // A normal desktop launch has no console. Explicit CLI use can reuse
        // the invoking terminal without opening an extra window.
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--version" | "-V"))
    {
        println!("RuneTeX {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    for flag in ["--project", "--engine"] {
        if let Some(index) = args.iter().position(|arg| arg == flag) {
            let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) else {
                eprintln!("{flag} 뒤에 값을 지정하세요.");
                std::process::exit(2);
            };
            if flag == "--engine" && compiler::Engine::from_name(value).is_none() {
                eprintln!("지원하는 엔진: pdflatex, xelatex, lualatex");
                std::process::exit(2);
            }
            let path = std::path::Path::new(value);
            if flag == "--project"
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tex"))
                && !path.exists()
            {
                eprintln!("TeX 파일이 없습니다: {}", path.display());
                std::process::exit(2);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--compile-only") {
        let state = State::new();
        let result = compiler::compile(compiler::CompileRequest {
            project_dir: state.project_dir.clone(),
            main_file: state.main_file.clone(),
            engine: state.engine,
            auto_install_packages: state.auto_install_packages,
            generation: 0,
        });
        println!("{}", result.log);
        if let Some(path) = &result.pdf_path {
            println!(
                "PDF: {}\nPages: {}",
                path.display(),
                result.preview.as_ref().map_or(0, |doc| doc.pages.len())
            );
        }
        let success = result.pdf_path.is_some();
        drop(state);
        if !success {
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "windows")]
    rune::set_app_icon_resource(1);
    #[cfg(target_os = "macos")]
    if let Err(error) = rune::set_app_icon(include_bytes!("../assets/app-icon.png").to_vec()) {
        eprintln!("앱 아이콘을 읽지 못했습니다: {error}");
    }
    App::new(AppSpec {
        min_size: Some((820.0, 560.0)),
        families: (preferences::DEFAULT_UI_FAMILIES, preferences::CODE_FAMILIES),
        setup: Some(State::setup),
        poll: Some(State::poll),
        on_key: Some(ui::on_key),
        ..AppSpec::new("RuneTeX", (1440.0, 940.0), State::new, ui::view)
    })
    .on_close(State::flush_on_close)
    .on_quit(State::flush_on_close)
    .run();
}
