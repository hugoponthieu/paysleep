use proton_drive::Client;

fn main() {
    let binary = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "proton-drive".into());
    match Client::new(binary).and_then(|client| client.probe()) {
        Ok(probe) => println!("{}", probe.version_output),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
