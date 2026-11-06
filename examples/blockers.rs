//! Every project in a folder that doesn't prepare natively, with each reason once:
//! `cargo run --release --example blockers -- <workshop folder> [assets]`.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next().map(std::path::PathBuf::from) else {
        eprintln!("usage: blockers <projects folder> [assets folder]");
        std::process::exit(2);
    };
    let assets = args.next().map(std::path::PathBuf::from);
    let Ok(entries) = std::fs::read_dir(&root) else {
        eprintln!("can't read {}", root.display());
        std::process::exit(1);
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Ok(project) = apolline_scene::Project::open(&entry.path(), assets.as_deref()) else {
            continue;
        };
        if let Err(error) = project.prepare_pointed() {
            let error = error.to_string();
            let mut reasons: Vec<&str> = error
                .split(['\n', ';'])
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect();
            reasons.sort();
            reasons.dedup();
            println!(
                "{} :: {}",
                entry.file_name().to_string_lossy(),
                reasons.join(" | ")
            );
        }
    }
}
