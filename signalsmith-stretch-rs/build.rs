fn main() {
    let mut build = cc::Build::new();
    build.cpp(true);
    build.file("src/wrapper.cpp");
    build.include("."); // include crate root so signalsmith-stretch.h and signalsmith-linear/stft.h can be resolved

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        build.flag("/std:c++14"); // Signalsmith requires C++11 or higher; MSVC supports /std:c++14 or c++17
        build.flag("/EHsc");      // Enable standard C++ exception handling
        build.flag("/O2");        // Enable speed optimizations
    } else {
        build.std("c++11");
    }

    build.compile("signalsmith_stretch");

    println!("cargo:rerun-if-changed=src/wrapper.cpp");
    println!("cargo:rerun-if-changed=signalsmith-stretch.h");
}
