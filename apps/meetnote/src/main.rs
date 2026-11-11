fn main() {
    if !meetnote::desktop::run_command_line() {
        meetnote::desktop::run_legacy_desktop();
    }
}
