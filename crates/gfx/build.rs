//! Compile GLSL shaders to SPIR-V with glslc (shaderc).
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for name in ["sprite.vert", "sprite.frag"] {
        let src = format!("shaders/{name}");
        println!("cargo:rerun-if-changed={src}");
        let status = Command::new("glslc")
            .args(["--target-env=vulkan1.3", "-O", &src, "-o"])
            .arg(out.join(format!("{name}.spv")))
            .status()
            .expect("failed to run glslc (install shaderc)");
        assert!(status.success(), "glslc failed on {src}");
    }
}
