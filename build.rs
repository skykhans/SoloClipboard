fn main() {
    println!("cargo:rerun-if-changed=assets/clipboard.rc");
    println!("cargo:rerun-if-changed=assets/clipboard.ico");
    embed_resource::compile("assets/clipboard.rc", embed_resource::NONE)
        .manifest_required()
        .expect("无法嵌入剪贴板图标");
}
