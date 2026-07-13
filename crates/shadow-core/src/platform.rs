pub fn is_android() -> bool {
    std::path::Path::new("/system/bin/sh").exists()
}
