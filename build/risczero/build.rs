// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::fmt::Write as _;

fn main() {
    if cfg!(feature = "rebuild-fpvm") {
        let build_opts = {
            #[cfg(not(any(feature = "debug-guest-build", debug_assertions)))]
            let root_dir = {
                let cwd = std::env::current_dir().unwrap();
                cwd.parent()
                    .unwrap()
                    .parent()
                    .map(|d| d.to_path_buf())
                    .unwrap()
            };
            std::collections::HashMap::from([("kailua-fpvm", {
                let opts = risc0_build::GuestOptions::default();
                // Build a reproducible ELF file using docker under the release profile
                #[cfg(not(any(feature = "debug-guest-build", debug_assertions)))]
                let opts = {
                    let mut opts = opts;
                    opts.use_docker = Some(
                        risc0_build::DockerOptionsBuilder::default()
                            .root_dir(root_dir)
                            .build()
                            .unwrap(),
                    );
                    opts
                };
                // Disable dev-mode receipts from being validated inside the guest
                #[cfg(any(
                    feature = "disable-dev-mode",
                    not(any(feature = "debug-guest-build", debug_assertions))
                ))]
                let opts = {
                    let mut opts = opts;
                    opts.features.push(String::from("disable-dev-mode"));
                    opts
                };
                opts
            })])
        };

        //let is_docker = build_opts["kailua-fpvm"].use_docker.is_some();
        risc0_build::embed_methods_with_options(build_opts);

        let src_bin_path = get_source_bin_dir(false);
        let target_dir = get_target_dir();
        let target_bin_path = target_dir.join("kailua-fpvm.bin");
        let target_code_path = target_dir.join("methods.rs");
        // copy bin to fpvm/src
        std::fs::copy(src_bin_path, target_bin_path.clone()).unwrap();
        // compute image id
        let bin = std::fs::read(&target_bin_path).unwrap();
        let image_id = risc0_zkvm::compute_image_id(&bin).unwrap();
        // override the methods.rs file to point to the new binary
        let mut methods = String::new();
        writeln!(&mut methods, "pub const KAILUA_FPVM_ELF: &[u8] = include_bytes!(\"./kailua-fpvm.bin\");").unwrap();
        writeln!(&mut methods, "pub const KAILUA_FPVM_PATH: &str = \"./kailua-fpvm.bin\";").unwrap();
        writeln!(&mut methods, "pub const KAILUA_FPVM_ID: [u32; 8] = {:?};", image_id.as_words()).unwrap();
        std::fs::write(&target_code_path, &methods).unwrap();
    }

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=fpvm/src");
}

fn get_source_bin_dir(is_docker: bool) -> PathBuf {
    let profile = get_profile(is_docker);
    let pkg = risc0_build::get_package(env::var("CARGO_MANIFEST_DIR").unwrap());
    get_out_dir()
        .join(pkg.name)
        .join("kailua-fpvm")
        .join("riscv32im-risc0-zkvm-elf")
        .join(profile)
        .join("kailua-fpvm.bin")
}

fn get_target_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .join("risczero/src")
}

fn get_out_dir() -> PathBuf {
    // This code is based on https://docs.rs/cxx-build/latest/src/cxx_build/target.rs.html#10-49

    if let Some(target_dir) = env::var_os("CARGO_TARGET_DIR").map(Into::<PathBuf>::into) {
        if target_dir.is_absolute() {
            return target_dir.join("riscv-guest");
        }
    }

    let mut dir: PathBuf = env::var_os("OUT_DIR").unwrap().into();
    loop {
        if dir.join(".rustc_info.json").exists()
            || dir.join("CACHEDIR.TAG").exists()
            || dir.file_name() == Some(OsStr::new("target"))
            && dir
            .parent()
            .is_some_and(|parent| parent.join("Cargo.toml").exists())
        {
            return dir.join("riscv-guest");
        }
        if dir.pop() {
            continue;
        }
        panic!("Cannot find cargo target dir location")
    }
}

fn get_profile(use_docker: bool) -> &'static str {
    if use_docker {
        "docker"
    } else if get_env_var("RISC0_BUILD_DEBUG") == "1" {
        "debug"
    } else {
        "release"
    }
}

fn get_env_var(name: &str) -> String {
    let ret = env::var(name).unwrap_or_default();
    if let Some(pkg) = env::var_os("CARGO_PKG_NAME") {
        if pkg != "cargo-risczero" {
            println!("cargo:rerun-if-env-changed={name}");
        }
    }
    ret
}