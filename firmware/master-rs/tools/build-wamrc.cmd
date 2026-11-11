@echo off
rem Builds wamrc (WAMR 2.4.5 AOT compiler) with Espressif's LLVM 18: targets RISC-V (ESP32-C3)
rem and Xtensa (classic ESP32). Long: the LLVM build takes hours and ~20 GB in D:\CrabLogic\.llvm.
rem Needs: Visual Studio 2022 (C++ build tools), git, and the cmake/ninja that ESP-IDF put in .embuild.
rem Result: firmware\master-rs\.deps\wamrc-esp\wamrc.exe
setlocal
set ROOT=%~dp0..\..\..
pushd %ROOT%
set ROOT=%CD%
popd
set LLVM=%ROOT%\.llvm
set WAMR=%ROOT%\firmware\master-rs\.deps\wamr-full
set OUT=%ROOT%\firmware\master-rs\.deps\wamrc-esp
set PATH=%ROOT%\.embuild\espressif\tools\cmake\3.30.2\bin;%ROOT%\.embuild\espressif\tools\ninja\1.12.1;%PATH%

call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1

if not exist "%WAMR%\wamr-compiler" (
  git clone -q --depth 1 -b WAMR-2.4.5 https://github.com/bytecodealliance/wasm-micro-runtime "%WAMR%" || exit /b 1
)
rem The source comes as an archive: a git clone of llvm-project tends to break off on a slow link,
rem while curl resumes (-C -) where it stopped.
rem Only llvm/, cmake/ and third-party/ are unpacked: the clang, lldb and libclc folders hold
rem symlinks that tar cannot create on Windows, and wamrc needs none of them.
if not exist "%LLVM%\src\llvm\CMakeLists.txt" (
  if not exist "%LLVM%" mkdir "%LLVM%"
  if not exist "%LLVM%\src.tar.gz" (
    curl -L --retry 20 --retry-all-errors -C - -o "%LLVM%\src.tar.gz.part" ^
      https://github.com/espressif/llvm-project/archive/refs/heads/xtensa_release_18.1.2.tar.gz || exit /b 1
    move /y "%LLVM%\src.tar.gz.part" "%LLVM%\src.tar.gz" >nul || exit /b 1
  )
  if not exist "%LLVM%\src" mkdir "%LLVM%\src"
  rem Windows' own bsdtar: a GNU tar from Git Bash on PATH reads "D:" as a remote host.
  "%SystemRoot%\System32\tar.exe" -xzf "%LLVM%\src.tar.gz" -C "%LLVM%\src" --strip-components=1 ^
    "llvm-project-xtensa_release_18.1.2/llvm" "llvm-project-xtensa_release_18.1.2/cmake" ^
    "llvm-project-xtensa_release_18.1.2/third-party" || exit /b 1
)
rem Configure runs every time: cheap, and picks up option changes in an existing build.
(
  cmake -S "%LLVM%\src\llvm" -B "%LLVM%\build" -G Ninja ^
    -DCMAKE_BUILD_TYPE=Release ^
    -DLLVM_TARGETS_TO_BUILD="RISCV;X86" ^
    -DLLVM_EXPERIMENTAL_TARGETS_TO_BUILD=Xtensa ^
    -DLLVM_ENABLE_PROJECTS= ^
    -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF ^
    -DLLVM_ENABLE_BINDINGS=OFF -DLLVM_ENABLE_TERMINFO=OFF -DLLVM_ENABLE_LIBEDIT=OFF ^
    -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF ^
    -DLLVM_INCLUDE_DOCS=OFF -DLLVM_INCLUDE_UTILS=OFF -DLLVM_INCLUDE_TOOLS=OFF ^
    -DLLVM_ENABLE_DIA_SDK=OFF ^
    -DLLVM_BUILD_TOOLS=OFF -DLLVM_OPTIMIZED_TABLEGEN=ON || exit /b 1
)
cmake --build "%LLVM%\build" || exit /b 1

cmake -S "%WAMR%\wamr-compiler" -B "%LLVM%\wamrc" -G Ninja -DCMAKE_BUILD_TYPE=Release ^
  -DWAMR_BUILD_WITH_CUSTOM_LLVM=1 -DLLVM_DIR="%LLVM%\build\lib\cmake\llvm" || exit /b 1
cmake --build "%LLVM%\wamrc" || exit /b 1

if not exist "%OUT%" mkdir "%OUT%"
copy /y "%LLVM%\wamrc\wamrc.exe" "%OUT%\" >nul || exit /b 1
"%OUT%\wamrc.exe" --version
echo wamrc ready: %OUT%\wamrc.exe
