//! clasificador.exe — entry point GUI con eframe + egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clasificador::gui::ClasificadorApp;

#[cfg(windows)]
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, FALSE, HANDLE, HWND, LPARAM, TRUE,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};
#[cfg(windows)]
use windows::core::{w, PWSTR};

use clasificador::gui::theme;
use egui::{Color32, RichText};

/// Garantiza que solo una instancia del .exe esté activa al tiempo.
///
/// Crea un Named Mutex global con un nombre único de la app. Si ya existe
/// (otra instancia lo creó antes), se muestra un MessageBox y se sale con
/// código 0. El mutex se libera automáticamente cuando este proceso termina
/// (incluso por crash o kill desde Task Manager) — el sistema operativo se
/// encarga, no necesitamos cleanup manual.
///
/// El handle devuelto debe vivir todo el `main()` para que el mutex siga
/// tomado. Cuando `main` retorna y el handle se dropea, el OS libera el
/// mutex y la siguiente instancia podrá tomarlo.
#[cfg(windows)]
fn ensure_single_instance() -> Option<HANDLE> {
    // Prefijo "Local\" → mutex visible solo en la sesión del usuario actual.
    // Sin prefijo o con "Global\" sería visible en otras sesiones del SO,
    // lo cual no queremos para una app de usuario.
    let mutex_name = w!("Local\\clasificador-mera-singleton-v1");

    let handle = unsafe { CreateMutexW(None, true, mutex_name) };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    match handle {
        Ok(h) if !already_exists => Some(h),
        _ => {
            show_already_running_window();
            None
        }
    }
}

/// Stub para no-Windows (Linux/macOS): no aplicamos lock, siempre permitimos.
#[cfg(not(windows))]
fn ensure_single_instance() -> Option<()> { Some(()) }

#[cfg(not(windows))]
fn focus_other_clasificador_window() -> bool { false }

/// Estado pasado al callback de EnumWindows. Se accede vía LPARAM como
/// puntero crudo; ese cast solo es seguro porque EnumWindows es síncrono.
#[cfg(windows)]
struct FocusState {
    my_pid: u32,
    found:  Option<HWND>,
}

/// Recorre todas las ventanas top-level del sistema buscando una que
/// (a) sea visible, (b) pertenezca a un proceso DISTINTO al actual,
/// (c) ese proceso sea `clasificador.exe`. Si la encuentra, la guarda
/// en `state.found` y detiene la enumeración devolviendo FALSE.
#[cfg(windows)]
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut FocusState);
    if state.found.is_some() { return FALSE; }
    if !IsWindowVisible(hwnd).as_bool() { return TRUE; }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == state.my_pid { return TRUE; }

    let h = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
        Ok(h) => h,
        Err(_) => return TRUE,
    };

    // QueryFullProcessImageNameW devuelve "C:\…\clasificador.exe".
    let mut buf = [0u16; 520];
    let mut size: u32 = buf.len() as u32;
    let res = QueryFullProcessImageNameW(
        h,
        PROCESS_NAME_FORMAT(0),  // 0 = Win32 path
        PWSTR(buf.as_mut_ptr()),
        &mut size,
    );
    let _ = CloseHandle(h);

    if res.is_ok() && size > 0 {
        let path = String::from_utf16_lossy(&buf[..size as usize]).to_lowercase();
        if path.ends_with("\\clasificador.exe") {
            state.found = Some(hwnd);
            return FALSE; // detener enumeración
        }
    }
    TRUE
}

/// Encuentra la ventana de la otra instancia de `clasificador.exe`,
/// la restaura si está minimizada y la trae al frente.
/// Retorna true si lo logró, false si no había ninguna candidata.
#[cfg(windows)]
fn focus_other_clasificador_window() -> bool {
    let my_pid = unsafe { GetCurrentProcessId() };
    let mut state = FocusState { my_pid, found: None };

    unsafe {
        // El Result de EnumWindows refleja si el callback terminó normalmente
        // o pidió detenerse. Para nosotros ambos son OK; lo importante es el
        // contenido de `state.found`.
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut state as *mut FocusState as isize),
        );
    }

    if let Some(hwnd) = state.found {
        unsafe {
            // Si la otra instancia está minimizada, primero la restauramos.
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // SetForegroundWindow tiene políticas de Windows que pueden
            // hacerla titilar en taskbar en vez de robar foco bruscamente
            // — es lo correcto y deseable como UX.
            let _ = SetForegroundWindow(hwnd);
        }
        true
    } else {
        false
    }
}

/// Mini-ventana eframe que avisa "ya está abierto" usando la misma paleta
/// e icono que la GUI principal. Bloquea hasta que el usuario la cierra
/// (X de la barra o botón "Entendido").
fn show_already_running_window() {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([460.0, 230.0])
        .with_min_inner_size([460.0, 230.0])
        .with_resizable(false)
        .with_title("Clasificador");
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // Ignoramos el Result — si falla mostrar la ventana, no podemos hacer
    // mucho más; el comportamiento "ya hay otra abierta" se cumple igual
    // (nuestra instancia muere al retornar de main).
    let _ = eframe::run_native(
        "clasificador-singleton-warn",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(SingletonWarnApp::default()))
        }),
    );
}

/// App eframe minimalista para el aviso de instancia duplicada.
#[derive(Default)]
struct SingletonWarnApp;

impl eframe::App for SingletonWarnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Banda verde fina pegada al borde superior — misma seña visual
        // que el header de la GUI principal.
        egui::TopBottomPanel::top("singleton_top_band")
            .exact_height(3.0)
            .frame(egui::Frame::default().fill(theme::ACCENT))
            .show(ctx, |_| {});

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG).inner_margin(22.0))
            .show(ctx, |ui| {
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    // Icono de aviso a la izquierda — usamos un emoji
                    // con color WARN del theme para que destaque.
                    ui.label(RichText::new("⚠").size(38.0).color(theme::WARN));
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Ya está abierto")
                            .size(18.0).strong().color(theme::ACCENT_H));
                        ui.add_space(6.0);
                        ui.label(RichText::new(
                            "El Clasificador de repositorios ya tiene una \
                             instancia activa.\n\n\
                             Busca su ventana en la barra de tareas (icono \
                             verde) o usa Alt+Tab.\n\nSi crees que se quedó \
                             colgada, ciérrala desde el Administrador de tareas."
                        ).size(12.0).color(theme::FG));
                    });
                });

                ui.add_space(18.0);

                // Botón "Llevarme allí" alineado a la derecha. Trae la
                // ventana de la otra instancia al frente y cierra esta.
                // Si la otra ventana está minimizada, la restaura primero.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn = egui::Button::new(
                        RichText::new("Ir a la ventana abierta").color(Color32::WHITE)
                    ).fill(theme::ACCENT);
                    if ui.add_sized([170.0, 30.0], btn).clicked() {
                        let _ = focus_other_clasificador_window();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.add_space(8.0);
                    if ui.add_sized([90.0, 30.0],
                        egui::Button::new(RichText::new("Cerrar").color(theme::FG))
                    ).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
    }
}

/// Carga el icono empaquetado en `clasificador.ico` para la ventana eframe.
fn load_window_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../clasificador.ico");
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width: w,
        height: h,
    })
}

/// Instala un panic hook global que escribe a `crash.log` en el directorio
/// del .exe ANTES de que `panic = "abort"` termine el proceso. Captura el
/// mensaje, ubicación y backtrace de cualquier thread (main + workers).
fn install_panic_hook() {
    // Forzar backtrace completo aunque el usuario no haya seteado RUST_BACKTRACE.
    std::env::set_var("RUST_BACKTRACE", "full");

    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let msg: String = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<panic payload no decodificable>".to_string()
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<sin ubicación>".to_string());

        let thread_name = std::thread::current()
            .name()
            .unwrap_or("<sin nombre>")
            .to_string();

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let backtrace = std::backtrace::Backtrace::force_capture();

        let log_text = format!(
            "================================================================\n\
             PANIC capturado\n\
             ----------------------------------------------------------------\n\
             Timestamp:  {}\n\
             Thread:     {}\n\
             Mensaje:    {}\n\
             Ubicación:  {}\n\
             ----------------------------------------------------------------\n\
             Backtrace:\n{}\n\
             ================================================================\n",
            timestamp, thread_name, msg, location, backtrace
        );

        // Escribir a crash.log junto al .exe.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let crash_path = dir.join("crash.log");
                // Si ya existe, anexar; sino crear.
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&crash_path)
                {
                    let _ = f.write_all(log_text.as_bytes());
                }
            }
        }

        // También a stderr por si el .exe se lanzara desde una consola.
        eprintln!("{}", log_text);
    }));
}

fn main() -> eframe::Result {
    install_panic_hook();

    // Single-instance: si ya hay otro clasificador.exe corriendo, mostramos
    // mensaje y salimos. El _mutex_handle DEBE vivir todo el main() — al
    // dropearse libera el mutex global y permite la próxima instancia.
    let _mutex_handle = match ensure_single_instance() {
        Some(h) => h,
        None    => return Ok(()),
    };

    // Sin texto en la barra de Windows: solo se ve el icono. El nombre
    // de la app ya está como <h1> dentro de la GUI, no hace falta repetirlo.
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1260.0, 740.0])
        .with_min_inner_size([1100.0, 600.0])
        .with_title("");
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        // app_name (storage key interno de eframe) — no afecta lo visible.
        "clasificador",
        options,
        Box::new(|cc| {
            clasificador::gui::theme::apply(&cc.egui_ctx);
            Ok(Box::new(ClasificadorApp::default()))
        }),
    )
}
