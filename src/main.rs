use clap::{Parser, Subcommand};
use hero_sms_mcp::{
    MAX_BODY, api, catalog, native,
    protocol::{self, RequestContext},
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser)]
#[command(
    version,
    about = "HeroSMS CLI and MCP server. Credentials: HEROSMS_API_KEY for CLI/stdio; X-HeroSMS-API-Key for HTTP."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    List,
    Describe {
        operation: String,
    },
    Call {
        operation: String,
        #[arg(long, default_value = "{}")]
        args: String,
        #[arg(long)]
        yes: bool,
    },
    Stdio,
    Serve {
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
}
fn key() -> Option<String> {
    std::env::var("HEROSMS_API_KEY").ok()
}
#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), &'static str> {
    match Cli::parse().command {
        Command::List => {
            for op in catalog::operations() {
                println!(
                    "{}{}",
                    op.name,
                    if op.mutating { " (mutating)" } else { "" }
                );
            }
        }
        Command::Describe { operation } => {
            let op = catalog::find(&operation).ok_or("Unknown operation")?;
            println!(
                "{}",
                json!({"name":op.name,"operationId":op.operation_id,"method":op.method,"path":op.path,"mutating":op.mutating,"inputSchema":op.input_schema})
            );
        }
        Command::Call {
            operation,
            args,
            yes,
        } => {
            let op = catalog::find(&operation).ok_or("Unknown operation")?;
            if op.mutating && !yes {
                return Err(
                    "This operation changes account state or spends balance; pass --yes to confirm",
                );
            }
            if args.len() > MAX_BODY {
                return Err("Arguments too large");
            }
            let args: Value = serde_json::from_str(&args).map_err(|_| "Invalid argument JSON")?;
            let key = key().ok_or("HEROSMS_API_KEY is required")?;
            let prepared = api::prepare_call(&operation, args, &key)?;
            let result = api::tool_result(native::execute(prepared).await, &key);
            println!("{result}");
            if result["isError"] == true {
                return Err("HeroSMS call failed");
            }
        }
        Command::Serve { port } => {
            native::serve(port).await?;
        }
        Command::Stdio => {
            let context = RequestContext::new(key());
            let mut reader = BufReader::new(tokio::io::stdin());
            let mut output = tokio::io::stdout();
            loop {
                let mut bytes = Vec::new();
                // take() bounds allocation even when a client omits its newline.
                let n = tokio::io::AsyncReadExt::take(&mut reader, (MAX_BODY + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                    .await
                    .map_err(|_| "Stdin failed")?;
                if n == 0 {
                    break;
                }
                if bytes.len() > MAX_BODY {
                    return Err("Stdio message too large");
                }
                let response = match serde_json::from_slice(&bytes) {
                    Ok(request) => protocol::dispatch(request, &context).await,
                    Err(_) => Some(protocol::error(Value::Null, -32700, "Parse error")),
                };
                if let Some(response) = response {
                    output
                        .write_all(format!("{response}\n").as_bytes())
                        .await
                        .map_err(|_| "Stdout failed")?;
                    output.flush().await.map_err(|_| "Stdout failed")?;
                }
            }
        }
    }
    Ok(())
}
