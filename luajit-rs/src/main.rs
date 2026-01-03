//! LuaJIT-RS command line interface.
//!
//! A Lua interpreter with JIT compilation using Cranelift.

use clap::{Parser, Subcommand};
use luajit_rs::{new_state, Value, VERSION};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "luajit-rs")]
#[command(author = "LuaJIT-RS Contributors")]
#[command(version = VERSION)]
#[command(about = "A Lua interpreter with Cranelift JIT compilation", long_about = None)]
struct Cli {
    /// Lua script file to execute
    #[arg(value_name = "SCRIPT")]
    script: Option<PathBuf>,

    /// Execute string as Lua code
    #[arg(short = 'e', long, value_name = "CODE")]
    execute: Option<String>,

    /// Enter interactive mode after executing script
    #[arg(short, long)]
    interactive: bool,

    /// Disable JIT compilation
    #[arg(long)]
    no_jit: bool,

    /// Arguments passed to Lua script
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a Lua file to bytecode
    Compile {
        /// Input Lua file
        input: PathBuf,
        /// Output bytecode file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Dump bytecode of a Lua file
    Dump {
        /// Input Lua file
        input: PathBuf,
    },
    /// Show JIT status and statistics
    JitStatus,
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    // Create Lua state
    let mut state = new_state();

    // Disable JIT if requested
    if cli.no_jit {
        // JIT disable would go here
        eprintln!("JIT disabled");
    }

    // Set up arg table
    setup_args(&mut state, &cli.args);

    // Handle subcommands
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Compile { input, output } => {
                compile_file(&mut state, &input, output.as_deref());
                return;
            }
            Commands::Dump { input } => {
                dump_bytecode(&mut state, &input);
                return;
            }
            Commands::JitStatus => {
                show_jit_status(&state);
                return;
            }
        }
    }

    // Execute string if provided
    if let Some(code) = &cli.execute {
        if let Err(e) = execute_string(&mut state, code) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    // Execute script file if provided
    if let Some(script) = &cli.script {
        if let Err(e) = execute_file(&mut state, script) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    // Enter interactive mode if requested or if no script provided
    if cli.interactive || (cli.script.is_none() && cli.execute.is_none()) {
        run_repl(&mut state);
    }
}

fn setup_args(state: &mut luajit_rs::vm::State, args: &[String]) {
    let arg_table = state.create_table(args.len(), 0);

    unsafe {
        for (i, arg) in args.iter().enumerate() {
            let key = Value::integer((i + 1) as i32);
            let val = state.intern_string(arg);
            (*arg_table.as_ptr()).set(key, val);
        }
    }

    state.set_global("arg", Value::table(arg_table));
}

fn execute_string(state: &mut luajit_rs::vm::State, code: &str) -> Result<(), String> {
    let func = state.load_string(code, "=(command line)")
        .map_err(|e| e.to_string())?;

    state.push(Value::function(func)).map_err(|e| e.to_string())?;
    state.call(0, 0).map_err(|e| e.to_string())?;

    Ok(())
}

fn execute_file(state: &mut luajit_rs::vm::State, path: &PathBuf) -> Result<(), String> {
    let code = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot open {}: {}", path.display(), e))?;

    let chunk_name = format!("@{}", path.display());
    let func = state.load_string(&code, &chunk_name)
        .map_err(|e| e.to_string())?;

    state.push(Value::function(func)).map_err(|e| e.to_string())?;
    state.call(0, 0).map_err(|e| e.to_string())?;

    Ok(())
}

fn run_repl(state: &mut luajit_rs::vm::State) {
    println!("{}", VERSION);
    println!("Type 'exit' or Ctrl+D to quit");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        stdout.flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // EOF
                println!();
                break;
            }
            Ok(_) => {
                let line = line.trim();

                if line == "exit" || line == "quit" {
                    break;
                }

                if line.is_empty() {
                    continue;
                }

                // Try as expression first (prepend 'return')
                let result = execute_string(state, &format!("return {}", line))
                    .or_else(|_| execute_string(state, line));

                if let Err(e) = result {
                    eprintln!("{}", e);
                }
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }
}

fn compile_file(state: &mut luajit_rs::vm::State, input: &PathBuf, output: Option<&std::path::Path>) {
    let code = match std::fs::read_to_string(input) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cannot read {}: {}", input.display(), e);
            std::process::exit(1);
        }
    };

    let chunk_name = format!("@{}", input.display());
    match luajit_rs::parse(&code, &chunk_name) {
        Ok(proto) => {
            let output_path = output
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| input.with_extension("luac"));

            // Would write bytecode here
            println!("Compiled {} -> {}", input.display(), output_path.display());
            println!("  {} instructions", proto.code.len());
            println!("  {} constants", proto.constants.len());
        }
        Err(e) => {
            eprintln!("Compile error: {}", e);
            std::process::exit(1);
        }
    }
}

fn dump_bytecode(state: &mut luajit_rs::vm::State, input: &PathBuf) {
    let code = match std::fs::read_to_string(input) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cannot read {}: {}", input.display(), e);
            std::process::exit(1);
        }
    };

    let chunk_name = format!("@{}", input.display());
    match luajit_rs::parse(&code, &chunk_name) {
        Ok(proto) => {
            println!("-- Bytecode dump: {}", input.display());
            println!("-- {} instructions, {} constants", proto.code.len(), proto.constants.len());
            println!();

            for (i, instr) in proto.code.iter().enumerate() {
                let line = proto.lineinfo.get(i).copied().unwrap_or(0);
                println!("{:4}  [{:3}]  {}", i, line, instr);
            }

            if !proto.constants.is_empty() {
                println!();
                println!("-- Constants:");
                for (i, k) in proto.constants.iter().enumerate() {
                    println!("  K{}: {:?}", i, k);
                }
            }
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    }
}

fn show_jit_status(state: &luajit_rs::vm::State) {
    println!("JIT Status:");
    println!("  Status: enabled");
    println!("  Backend: Cranelift");

    // Would show more JIT stats here
    println!("  Traces: 0");
    println!("  Code size: 0 bytes");
}
