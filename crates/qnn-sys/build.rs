use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=QNN_SDK_ROOT");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));

    let sdk_root = env::var("QNN_SDK_ROOT").ok();

    match sdk_root {
        Some(root) if !root.is_empty() => {
            let root = PathBuf::from(root);
            generate_real_bindings(&root, &out_dir);
            link_qnn(&root);
        }
        _ => {
            if cfg!(feature = "require-sdk") {
                panic!(
                    "QNN_SDK_ROOT is not set. Install the Qualcomm AI Engine Direct SDK \
                     (QNN SDK 2.44+) from the Qualcomm developer portal and set \
                     QNN_SDK_ROOT to its install path. See scripts/setup-qnn.ps1."
                );
            }
            println!(
                "cargo:warning=QNN_SDK_ROOT not set; emitting stub bindings. \
                 qnn-sys will not link against the real QNN runtime. \
                 Set QNN_SDK_ROOT to enable NPU functionality."
            );
            emit_stub_bindings(&out_dir);
        }
    }
}

fn generate_real_bindings(sdk_root: &Path, out_dir: &Path) {
    let qnn_include = sdk_root.join("include").join("QNN");
    let genie_include = sdk_root.join("include").join("Genie");
    if !qnn_include.exists() {
        panic!(
            "QNN_SDK_ROOT={} does not contain include/QNN. \
             Verify the SDK install layout.",
            sdk_root.display()
        );
    }
    if !genie_include.exists() {
        panic!(
            "QNN_SDK_ROOT={} does not contain include/Genie. \
             Verify the SDK install layout (need QAIRT 2.44+ for Genie).",
            sdk_root.display()
        );
    }

    let header = out_dir.join("wrapper.h");
    std::fs::write(
        &header,
        r#"// Auto-generated umbrella header for bindgen.

// QNN core
#include "QNN/QnnCommon.h"
#include "QNN/QnnTypes.h"
#include "QNN/QnnInterface.h"
#include "QNN/QnnBackend.h"
#include "QNN/QnnContext.h"
#include "QNN/QnnGraph.h"
#include "QNN/QnnTensor.h"
#include "QNN/QnnDevice.h"
#include "QNN/QnnLog.h"
#include "QNN/QnnProfile.h"
#include "QNN/QnnMem.h"
#include "QNN/QnnProperty.h"
#include "QNN/System/QnnSystemContext.h"

// Genie LLM runtime (higher-level wrapper over QNN HTP)
#include "Genie/GenieCommon.h"
#include "Genie/GenieDialog.h"
"#,
    )
    .expect("write wrapper.h");

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        // Both `-I include` (so umbrella header's "QNN/Foo.h" resolves) and
        // `-I include/QNN` (so internal cross-includes like "QnnDevice.h"
        // without the prefix resolve too).
        .clang_arg(format!("-I{}", sdk_root.join("include").display()))
        .clang_arg(format!("-I{}", qnn_include.display()))
        .clang_arg(format!("-I{}", genie_include.display()))
        .allowlist_function("Qnn.*")
        .allowlist_function("Genie.*")
        .allowlist_function("Genie_.*")
        .allowlist_type("Qnn.*")
        .allowlist_type("Genie.*")
        .allowlist_type("Genie_.*")
        .allowlist_var("QNN_.*")
        .allowlist_var("GENIE_.*")
        .derive_default(true)
        .derive_debug(true)
        .layout_tests(false)
        .generate()
        .expect("generate QNN + Genie bindings");

    let out = out_dir.join("bindings.rs");
    bindings.write_to_file(&out).expect("write bindings.rs");
}

fn link_qnn(sdk_root: &Path) {
    // QAIRT does not ship a QnnSystem.lib import library on Windows ARM64
    // (only QnnSystem.dll). The QNN core is loaded via runtime dynamic
    // dispatch (libloading), not a static link.
    //
    // Genie.lib *does* ship, so the Genie LLM runtime is statically linked
    // here -- no runtime loading dance required for the LLM path.
    let arch_dir = sdk_root.join("lib").join("aarch64-windows-msvc");
    if arch_dir.exists() {
        println!("cargo:rustc-link-search=native={}", arch_dir.display());
        println!("cargo:rerun-if-changed={}", arch_dir.display());
        // Link Genie statically (Genie.lib + Genie.dll at runtime).
        println!("cargo:rustc-link-lib=dylib=Genie");
    } else {
        println!(
            "cargo:warning=Expected QNN lib dir not found: {}. \
             Adjust qnn-sys/build.rs for your SDK layout.",
            arch_dir.display()
        );
    }
}

fn emit_stub_bindings(out_dir: &Path) {
    let stub = r#"// Stub bindings emitted because QNN_SDK_ROOT was not set.
// Building qnn-sys with the real SDK requires QNN_SDK_ROOT to point at a
// QNN SDK 2.44+ install. Without it, this crate exposes no FFI symbols and
// any attempt to use it at runtime will panic.

pub const QNN_SDK_ROOT_NOT_SET: bool = true;
"#;
    std::fs::write(out_dir.join("bindings.rs"), stub).expect("write stub bindings.rs");
}
