use proton_drive::{Client, CredentialsStore};

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let binary = arguments.next().unwrap_or_else(|| "proton-drive".into());
    let state_dir = arguments.next().unwrap_or_else(|| "./proton-state".into());

    if let Err(error) = Client::new(binary)
        .map(|client| {
            client
                .with_cache_dir(state_dir)
                .with_credentials_store(CredentialsStore::Keychain)
        })
        .and_then(|client| client.login_interactive())
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
