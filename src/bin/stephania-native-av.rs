//! Standalone entry point for the shared native A/V publisher.
fn main() {
    if let Err(e) = stephania::native_av::run_from_env() {
        eprintln!("native A/V stopped ({:?})", e.kind());
        std::process::exit(1);
    }
}
