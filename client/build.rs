#[cfg(target_os = "windows")]
fn main() {
    use winres::WindowsResource;

    let mut res = WindowsResource::new();
    res.set_icon("assets/hang_icon.ico");
    res.compile().expect("Failed to embed Windows icon");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
