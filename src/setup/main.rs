#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
#[path = "../icons.rs"]
mod icons;
mod installer;
mod payload;
mod platform;
mod state;
mod ui;

use rune::{App, AppSpec};
use state::SetupState;

fn main() {
    #[cfg(windows)]
    if std::env::args().len() > 1 {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    match cli::run() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            #[cfg(windows)]
            if !std::env::args()
                .any(|argument| argument == "--silent" || argument == "--package-info")
            {
                let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
                let title: Vec<u16> = "RuneTeX".encode_utf16().chain(Some(0)).collect();
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                        std::ptr::null_mut(),
                        message.as_ptr(),
                        title.as_ptr(),
                        windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                            | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                    );
                }
            }
            std::process::exit(1);
        }
    }
    #[cfg(windows)]
    rune::set_app_icon_resource(1);
    App::new(AppSpec {
        min_size: Some((940.0, 480.0)),
        families: (
            "Segoe UI, Malgun Gothic, Inter",
            "Cascadia Code, Consolas, Malgun Gothic",
        ),
        poll: Some(SetupState::poll),
        setup: Some(SetupState::setup),
        on_key: Some(ui::on_key),
        ..AppSpec::new(
            if std::env::args().any(|arg| arg == "--preview" || arg.starts_with("--screen")) {
                "RuneTeX 설치 · 디자인 미리보기"
            } else if std::env::args().any(|arg| arg == "--uninstall") {
                "RuneTeX 제거"
            } else {
                "RuneTeX 설치"
            },
            (1120.0, 480.0),
            SetupState::new,
            ui::view,
        )
    })
    .on_close(SetupState::close)
    .on_quit(SetupState::close)
    .run();
}
