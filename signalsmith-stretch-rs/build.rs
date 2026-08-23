fn main() {
    cc::Build::new()
        .cpp(true)
        .std("c++11")
        .file("src/wrapper.cpp")
        .include(".") // include crate root so signalsmith-stretch.h and signalsmith-linear/stft.h can be resolved
        .compile("signalsmith_stretch");

    println!("cargo:rerun-if-changed=src/wrapper.cpp");
    println!("cargo:rerun-if-changed=signalsmith-stretch.h");
}
