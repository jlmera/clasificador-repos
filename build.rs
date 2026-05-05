//! Embeber el icono `clasificador.ico` en el ejecutable Windows
//! como recurso, para que el .exe muestre el icono en Explorador.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("clasificador.ico");
        if let Err(e) = res.compile() {
            eprintln!("warning: winres failed to compile resources: {}", e);
        }
    }
}
