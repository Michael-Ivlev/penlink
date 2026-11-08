//! Read-only: prints each monitor's name, size and Windows DPI percent.
fn main() {
    penlink_host::winutil::init_process();
    for m in penlink_host::vdisplay::enum_monitors() {
        println!("{} {}x{} dpi={:?}", m.name, m.rect.w, m.rect.h, penlink_host::vdisplay::dpi_percent(&m.name));
    }
}
