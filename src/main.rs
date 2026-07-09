mod cli;
mod config;
mod context_export;
mod cycle;
mod dependency_graph;
mod graph_check;
mod handlers;
#[allow(dead_code)]
mod hooks;
mod index;
mod init;
mod link_gen;
mod metadata;
mod metadata_schema;
mod note_info;
mod note_ops;
mod parser;
mod reconcile;
mod server;
mod watcher;

use clap::Parser;
use tokio::sync::RwLock;
use tower_lsp::{LspService, Server};
use tracing_subscriber::{fmt, EnvFilter};

use cli::{Cli, Command, ConfigCommand, MetadataCommand};
use config::WikiConfig;
use server::ZkLspServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Tracing writes to stderr (stdout reserved for JSON-RPC)
    fmt()
        .with_env_filter(
            EnvFilter::try_from_env("ZK_LSP_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // `init` defaults to $PWD, not ~/wiki, so resolve its config before the
    // shared config (which defaults to ~/wiki for everything else).
    if let Some(Command::Init { id }) = &cli.command {
        let id = id.clone();
        let root = cli.wiki_root.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
        let config = WikiConfig::from_root(root);
        return init::init_wiki(&config, id).await;
    }

    let config = std::sync::Arc::new(WikiConfig::resolve(cli.wiki_root.clone(), None));

    match cli.command.unwrap_or(Command::Lsp) {
        Command::Lsp => {
            run_lsp(cli.wiki_root).await?;
        }
        Command::Generate => {
            link_gen::generate_link_typ(&config).await?;
            eprintln!("link.typ regenerated at {}", config.link_file.display());
        }
        Command::New { id, meta, json } => {
            let mut overrides = note_ops::MetaOverrides::new();
            let mut title = None;
            let mut body = None;
            if json {
                use std::io::Read;
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                let (json_meta, json_title, json_body) =
                    note_ops::parse_json_creation_input(&s, &config.zk_config)?;
                overrides = json_meta;
                title = json_title;
                body = json_body;
            }
            // --meta flags override JSON metadata
            for (k, v) in note_ops::parse_meta_overrides(&meta, &config.zk_config)? {
                overrides.insert(k, v);
            }
            let path = note_ops::create_note(&config, id, &overrides, title, body).await?;
            println!("{}", path.display());
        }
        Command::Remove { id } => {
            note_ops::delete_note(&id, &config).await?;
            eprintln!("Note {id} removed.");
        }
        Command::Format => {
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            let formatted = handlers::formatting::format_content(&content, &config).await?;
            print!("{formatted}");
        }
        Command::Reconcile { dry_run } => match reconcile::run_reconcile(&config, dry_run).await {
            Ok(stats) => eprintln!("Reconcile: {} file(s) changed", stats.files_changed),
            Err(err) => {
                eprint!("{err}");
                if !err.to_string().ends_with('\n') {
                    eprintln!();
                }
                std::process::exit(1);
            }
        },
        Command::Export {
            id,
            depth,
            inverse,
            simple,
        } => {
            let out = context_export::export_context(&id, depth, inverse, simple, &config).await?;
            print!("{out}");
        }
        Command::Init { .. } => unreachable!("handled above"),
        Command::Check {
            no_orphans,
            no_dead_links,
        } => {
            let mut report = graph_check::check_graph(&config).await?;
            let has_dead_links = !report.dead_links.is_empty();
            if no_dead_links {
                report.dead_links.clear();
            }
            if no_orphans {
                report.orphans.clear();
            }
            let rendered = graph_check::render_check_report(&report);
            print!("{rendered}");
            if has_dead_links && !no_dead_links {
                std::process::exit(1);
            }
        }
        Command::Notes { json: _ } => {
            let json = note_info::build_notes_json(&config).await?;
            println!("{json}");
        }
        Command::NoteInfo { id, json: _ } => {
            let json = note_info::build_single_note_info_json(&id, &config).await?;
            println!("{json}");
        }
        Command::Config { command } => {
            let out = render_config_command(&config, command)?;
            print!("{out}");
        }
    }
    Ok(())
}

fn render_config_command(config: &WikiConfig, command: ConfigCommand) -> anyhow::Result<String> {
    match command {
        ConfigCommand::Metadata { command } => match command {
            MetadataCommand::Fields { output } => metadata_schema::render_fields(
                config,
                metadata_output_format(output.toml),
                output.sources,
            ),
            MetadataCommand::Defaults { output } => metadata_schema::render_defaults(
                config,
                metadata_output_format(output.toml),
                output.sources,
            ),
            MetadataCommand::JsonSchema { output } => {
                metadata_schema::render_json_schema(config, output.sources)
            }
        },
    }
}

fn metadata_output_format(toml: bool) -> metadata_schema::MetadataSchemaFormat {
    if toml {
        metadata_schema::MetadataSchemaFormat::Toml
    } else {
        metadata_schema::MetadataSchemaFormat::Json
    }
}

async fn run_lsp(cli_root: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let config = std::sync::Arc::new(RwLock::new(WikiConfig::resolve(cli_root.clone(), None)));

    let (service, socket) = LspService::new(|client| ZkLspServer::new(client, config, cli_root));
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}
