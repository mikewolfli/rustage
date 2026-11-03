# The native bridge's headers

`napi_init.cpp` includes `rw_generated.h` from the pinned rust-widgets checkout, at the
path given by `RUST_WIDGETS_INCLUDE` (see `../CMakeLists.txt`). This directory is kept
so a project can drop a *symlink* to that header here without editing CMake:

```sh
ln -s <rust-widgets>/include/rw_generated.h \
  entry/src/main/cpp/include/rw_generated.h
```

A real copy is deliberately not shipped: the header is generated from the pinned
revision's binding sources, and a copy would be a second declaration of the same ABI
(principle #59).
