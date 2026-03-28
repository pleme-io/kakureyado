use clap::{Parser, Subcommand};
use kakureyado_core::{OnionServiceHost, ServiceConfig, ServiceRegistry, VanityGenerator};
use kakureyado_service::{BruteForceVanityGenerator, LocalOnionHost, MemoryRegistry};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "kakureyado",
    about = "Onion service hosting platform — expose services as .onion addresses",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create and start an onion service for a target
    Start {
        /// Service name
        #[arg(short, long)]
        name: String,
        /// Target address (e.g. 127.0.0.1)
        #[arg(short, long, default_value = "127.0.0.1")]
        target: String,
        /// Target port
        #[arg(short = 'p', long)]
        port: u16,
        /// Port exposed on the .onion address
        #[arg(short = 'o', long, default_value_t = 80)]
        onion_port: u16,
    },
    /// Stop a running onion service
    Stop {
        /// Service name
        name: String,
    },
    /// List all registered services
    List,
    /// Generate a vanity .onion address with a given prefix
    Vanity {
        /// Desired prefix (lowercase base32: a-z, 2-7)
        prefix: String,
    },
    /// Show the status of a service
    Status {
        /// Service name
        name: String,
    },
}

/// Execute CLI commands against the provided service components.
///
/// Extracted from `main` for testability.
async fn execute(
    host: &LocalOnionHost,
    registry: &MemoryRegistry,
    vanity_gen: &BruteForceVanityGenerator,
    command: Command,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Start {
            name,
            target,
            port,
            onion_port,
        } => {
            let config = ServiceConfig {
                name: name.clone(),
                target_addr: target,
                target_port: port,
                onion_port,
                persistent: true,
                vanity_prefix: None,
            };
            let svc = host.create_service(&config).await?;
            registry.register(svc).await?;
            host.start_service(&name).await?;
            let status = host.service_status(&name).await?;
            println!("Service '{name}' started — status: {status}");
        }
        Command::Stop { name } => {
            host.stop_service(&name).await?;
            println!("Service '{name}' stopped");
        }
        Command::List => {
            let services = registry.list().await?;
            if services.is_empty() {
                println!("No services registered.");
            } else {
                for svc in &services {
                    println!(
                        "{:<20} {:<62} {:>6} → {}:{} [{}]",
                        svc.name,
                        svc.onion_address,
                        svc.target_port,
                        svc.target_addr,
                        svc.target_port,
                        svc.status,
                    );
                }
            }
        }
        Command::Vanity { prefix } => {
            let estimate = vanity_gen.estimate_time(prefix.len());
            println!(
                "Generating vanity address with prefix '{prefix}' (estimated: {estimate:.1?})..."
            );
            let result = vanity_gen.generate(&prefix).await?;
            println!("Address:  {}", result.address);
            println!("Attempts: {}", result.attempts);
            println!("Duration: {:.2?}", result.duration);
        }
        Command::Status { name } => {
            let status = host.service_status(&name).await?;
            println!("Service '{name}': {status}");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let host = LocalOnionHost::new();
    let registry = MemoryRegistry::new();
    let vanity_gen = BruteForceVanityGenerator::default();

    execute(&host, &registry, &vanity_gen, cli.command).await
}
