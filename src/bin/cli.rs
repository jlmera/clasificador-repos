//! clasificador-cli — CLI explícito con clap.
//! Genera `tools/repos_index.json` y `buscador.html` en la raíz.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use clasificador::index::{rebuild_index, RebuildOpts};
use clasificador::html::generate_html;

#[derive(Parser, Debug)]
#[command(
    name = "clasificador-cli",
    version,
    about = "Clasificador de repositorios (CLI)"
)]
struct Cli {
    /// Carpeta raíz que contiene las categorías 0X-*\
    #[arg(long, default_value = r"D:\DUGOTEX\11 - IA\GitHub")]
    root: PathBuf,

    /// No intenta consultar GitHub API (offline)
    #[arg(long)]
    no_network: bool,

    /// Solo regenera índices, no procesa _inbox
    #[arg(long)]
    reindex_only: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    println!("=== CLASIFICADOR DE REPOSITORIOS (CLI) ===");
    println!("  root:        {}", cli.root.display());
    println!("  red:         {}", if cli.no_network { "no" } else { "sí (GitHub API)" });
    println!("  modo:        {}", if cli.reindex_only { "REINDEX-ONLY" } else { "FULL" });
    println!();

    println!("Re-escaneando todo el árbol…");
    // El CLI hereda el PAT del config cifrado de la GUI (mismo config.dat).
    let pat = if cli.no_network { None } else { clasificador::secrets::load_github_pat() };
    let data = match rebuild_index(&cli.root, RebuildOpts {
        allow_github: !cli.no_network,
        force_github_retry: false,
        github_pat: pat,
    }) {
        Ok(d) => d,
        Err(e) => { eprintln!("✗ Error reindexando: {}", e); return ExitCode::from(1); }
    };
    println!("  ✓ {} repos indexados", data.len());

    let sin_id  = data.iter().filter(|r| r.id.is_none()).count();
    let sin_es  = data.iter().filter(|r| r.necesita_traduccion).count();
    if sin_id > 0 { println!("  ⚠ {} repos sin GitHub ID", sin_id); }
    if sin_es > 0 { println!("  ⚠ {} repos sin traducción al español", sin_es); }

    match generate_html(&cli.root, &data) {
        Ok(p) => println!("  ✓ HTML generado: {}", p.display()),
        Err(e) => { eprintln!("✗ HTML: {}", e); return ExitCode::from(2); }
    }

    println!("✓ Reindexado completo.");
    ExitCode::SUCCESS
}
