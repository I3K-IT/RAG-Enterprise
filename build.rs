fn main() {
    // Embed the git short hash so the binary can identify itself at startup.
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=BUILD_GIT_HASH={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
}
