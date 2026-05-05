//! Catálogo de categorías y reglas heurísticas (palabras clave + pesos).
//! Idéntico al `KEYWORDS` y `CATEGORIAS` del Python.

pub const CATEGORIAS: &[&str] = &[
    "01-claude-code",
    "02-mcp-y-conectores",
    "03-agentes-y-llms",
    "04-listas-curadas",
    "05-self-hosted",
    "06-infraestructura-core",
    "07-multimedia-y-conversion",
    "08-productividad",
    "09-sistema-windows-linux",
    "10-utilidades-dev",
];

/// Palabras clave por categoría con su peso heurístico.
/// El score de un repo en una categoría = suma de (peso × ocurrencias)
/// de cada keyword en la combinación nombre+descripción+README.
pub const KEYWORDS: &[(&str, &[(&str, u32)])] = &[
    ("01-claude-code", &[
        ("claude code", 5), ("claude-code", 5), ("claude.md", 4),
        ("agent skill", 4), ("anthropic", 3), ("claude", 2),
        ("skill", 2), ("hooks", 2), ("status line", 3),
        ("plugin", 1), ("prompt", 1), ("token optimi", 3),
    ]),
    ("02-mcp-y-conectores", &[
        ("model context protocol", 5), ("mcp server", 5), ("mcp ", 3),
        ("mcp-", 3), ("connector", 2),
    ]),
    ("03-agentes-y-llms", &[
        ("rag", 4), ("retrieval-augmented", 5), ("llm", 3),
        ("ai agent", 4), ("agent ", 2), ("autonomous agent", 5),
        ("openai", 2), ("gpt-", 2), ("voice clone", 4),
        ("tutor", 2), ("agentic", 3),
    ]),
    ("04-listas-curadas", &[
        ("awesome ", 4), ("awesome-", 4), ("roadmap", 4),
        ("curated", 3), ("collection of", 2), ("book of", 4),
        ("guide", 1), ("self-hosting guide", 5),
    ]),
    ("05-self-hosted", &[
        ("self-host", 4), ("self host", 4), ("alternative to", 3),
        ("docker compose", 2), ("status page", 3), ("dashboard", 1),
        ("crm", 2), ("project management", 2), ("password manager", 4),
        ("email server", 4), ("document management", 3),
    ]),
    ("06-infraestructura-core", &[
        ("kernel", 4), ("operating system", 3),
        ("compiler", 3), ("interpreter", 3), ("runtime", 2),
        ("database", 2), ("dbms", 4), ("web server", 3),
    ]),
    ("07-multimedia-y-conversion", &[
        ("ocr", 5), ("pdf", 3), ("video", 3), ("audio", 2),
        ("voice", 2), ("tts", 4), ("stt", 4), ("convert", 1),
        ("compress", 2), ("transcribe", 4), ("ffmpeg", 3),
        ("markdown converter", 4), ("image", 1),
    ]),
    ("08-productividad", &[
        ("resume", 3), ("cv ", 3), ("career", 3), ("task manager", 4),
        ("todo", 3), ("productivity", 4), ("bookmark", 3),
        ("obsidian plugin", 4), ("workflow automation", 3),
    ]),
    ("09-sistema-windows-linux", &[
        ("wayland", 4), ("compositor", 3), ("network monitor", 4),
        ("sniff", 3), ("osint", 4), ("privacy", 3), ("anonymi", 3),
        ("debloat", 5), ("windows 11", 4), ("iptv", 5),
        ("sandbox", 3),
    ]),
    ("10-utilidades-dev", &[
        ("worktree", 5), ("ide ", 2), ("editor", 1), ("linter", 3),
        ("developer tool", 3), ("code editor", 4), ("git hook", 3),
        ("repo analy", 4), ("token counter", 3),
    ]),
];

/// Mapeo extensión → lenguaje (idéntico a Python).
pub fn lang_for_ext(ext: &str) -> Option<&'static str> {
    let ext = ext.to_lowercase();
    Some(match ext.as_str() {
        ".py" | ".pyc" => "Python",
        ".ts" | ".tsx" => "TypeScript",
        ".js" | ".jsx" | ".mjs" | ".cjs" => "JavaScript",
        ".go" => "Go",
        ".rs" => "Rust",
        ".c" | ".h" => "C",
        ".cc" | ".cpp" | ".hpp" => "C++",
        ".java" => "Java",
        ".rb" => "Ruby",
        ".php" => "PHP",
        ".cs" => "C#",
        ".sh" | ".bash" => "Shell",
        ".ps1" => "PowerShell",
        ".pas" => "Pascal",
        ".sql" => "SQL",
        ".astro" => "Astro",
        ".vue" => "Vue",
        ".svelte" => "Svelte",
        ".html" => "HTML",
        ".md" => "Markdown",
        _ => return None,
    })
}

pub const EXCLUDE_DIRS: &[&str] = &[
    "node_modules", ".git", "dist", "build", "vendor",
    "target", ".venv", "__pycache__", ".next", ".nuxt",
];

pub const TAG_KEYWORDS: &[(&str, &str)] = &[
    ("ai", "tema/ia"), ("claude", "tema/claude"), ("agent", "tema/agente"),
    ("rag", "tema/rag"), ("mcp", "tema/mcp"), ("ocr", "tema/ocr"),
    ("pdf", "tema/pdf"), ("video", "tema/video"), ("voice", "tema/voz"),
    ("self-host", "tema/self-hosted"), ("docker", "tema/docker"),
    ("kubernetes", "tema/k8s"), ("windows", "tema/windows"),
    ("linux", "tema/linux"), ("obsidian", "tema/obsidian"), ("n8n", "tema/n8n"),
];

/// Mapeo de topics oficiales de GitHub → (categoría, peso de boost).
///
/// Los topics son señales mucho más fuertes que las keywords del README:
/// el dueño del repo los puso intencionalmente como tags. Por eso usamos
/// pesos altos (5-10) cuando hay match.
///
/// Match es case-insensitive y exacto contra cada elemento del array
/// `topics` que devuelve GitHub API. Un topic compuesto tipo "claude-code"
/// matchea solo si el repo tiene EXACTAMENTE ese topic, no si tiene
/// "claude" y "code" por separado.
pub const TOPIC_BOOSTS: &[(&str, &str, u32)] = &[
    // 01-claude-code
    ("claude-code",       "01-claude-code", 10),
    ("claude",            "01-claude-code",  4),
    ("claude-skills",     "01-claude-code",  8),
    ("claude-md",         "01-claude-code",  8),
    ("anthropic",         "01-claude-code",  3),

    // 02-mcp-y-conectores
    ("model-context-protocol", "02-mcp-y-conectores", 10),
    ("mcp",                    "02-mcp-y-conectores",  8),
    ("mcp-server",             "02-mcp-y-conectores", 10),
    ("mcp-client",             "02-mcp-y-conectores",  8),

    // 03-agentes-y-llms
    ("ai-agent",          "03-agentes-y-llms", 7),
    ("ai-agents",         "03-agentes-y-llms", 7),
    ("agentic",           "03-agentes-y-llms", 6),
    ("agentic-ai",        "03-agentes-y-llms", 7),
    ("autonomous-agents", "03-agentes-y-llms", 7),
    ("llm",               "03-agentes-y-llms", 4),
    ("llms",              "03-agentes-y-llms", 4),
    ("rag",               "03-agentes-y-llms", 6),
    ("retrieval-augmented-generation", "03-agentes-y-llms", 8),
    ("openai",            "03-agentes-y-llms", 3),
    ("gpt",               "03-agentes-y-llms", 3),

    // 04-listas-curadas
    ("awesome",           "04-listas-curadas",  8),
    ("awesome-list",      "04-listas-curadas", 10),
    ("curated-list",      "04-listas-curadas",  8),
    ("roadmap",           "04-listas-curadas",  6),

    // 05-self-hosted
    ("self-hosted",       "05-self-hosted",  8),
    ("selfhosted",        "05-self-hosted",  8),
    ("docker-compose",    "05-self-hosted",  4),
    ("self-hosting",      "05-self-hosted",  6),
    ("password-manager",  "05-self-hosted",  6),
    ("dashboard",         "05-self-hosted",  3),
    ("crm",               "05-self-hosted",  4),

    // 06-infraestructura-core
    ("kernel",            "06-infraestructura-core", 6),
    ("operating-system",  "06-infraestructura-core", 6),
    ("compiler",          "06-infraestructura-core", 5),
    ("database",          "06-infraestructura-core", 5),
    ("web-server",        "06-infraestructura-core", 5),

    // 07-multimedia-y-conversion
    ("ocr",               "07-multimedia-y-conversion", 8),
    ("pdf",               "07-multimedia-y-conversion", 5),
    ("video",             "07-multimedia-y-conversion", 5),
    ("audio",             "07-multimedia-y-conversion", 4),
    ("speech-recognition","07-multimedia-y-conversion", 7),
    ("speech-to-text",    "07-multimedia-y-conversion", 7),
    ("text-to-speech",    "07-multimedia-y-conversion", 7),
    ("transcription",     "07-multimedia-y-conversion", 7),
    ("ffmpeg",            "07-multimedia-y-conversion", 5),
    ("image-processing",  "07-multimedia-y-conversion", 4),

    // 08-productividad
    ("resume",            "08-productividad", 5),
    ("cv",                "08-productividad", 4),
    ("productivity",      "08-productividad", 6),
    ("todo",              "08-productividad", 5),
    ("task-management",   "08-productividad", 6),
    ("bookmark",          "08-productividad", 5),
    ("obsidian-plugin",   "08-productividad", 7),

    // 09-sistema-windows-linux
    ("debloat",           "09-sistema-windows-linux", 8),
    ("windows-11",        "09-sistema-windows-linux", 7),
    ("wayland",           "09-sistema-windows-linux", 6),
    ("compositor",        "09-sistema-windows-linux", 5),
    ("network-monitor",   "09-sistema-windows-linux", 6),
    ("osint",             "09-sistema-windows-linux", 6),
    ("privacy",           "09-sistema-windows-linux", 4),
    ("iptv",              "09-sistema-windows-linux", 8),

    // 10-utilidades-dev
    ("developer-tools",   "10-utilidades-dev", 6),
    ("cli",               "10-utilidades-dev", 3),
    ("ide",               "10-utilidades-dev", 4),
    ("editor",            "10-utilidades-dev", 3),
    ("linter",            "10-utilidades-dev", 5),
    ("git-hook",          "10-utilidades-dev", 5),
    ("worktree",          "10-utilidades-dev", 7),
    ("repo-analysis",     "10-utilidades-dev", 6),
];
