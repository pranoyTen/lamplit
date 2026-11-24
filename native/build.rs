fn main() {
    slint_build::compile("ui/app.slint").expect("the UI should compile");
    // the icon on lamplit.exe itself (Explorer, the taskbar before the window sets its own, Alt+Tab)
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico").set("ProductName", "Lamplit").set("FileDescription", "Lamplit");
        res.compile().expect("the Windows icon resource should compile");
    }
}
