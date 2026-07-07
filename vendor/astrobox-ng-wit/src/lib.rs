wit_bindgen::generate!({
    path: "wit",
    world: "psys-world-v3",
    generate_all,
    pub_export_macro: true,
    default_bindings_module: "astrobox_ng_wit",
});

// Re-export common wit-bindgen runtime APIs so downstream crates
// don't need to depend on wit-bindgen directly.
pub use wit_bindgen::{
    block_on,
    spawn,
    FutureReader,
    FutureWriter,
    FutureRead,
    FutureWrite,
    FutureWriteCancel,
    FutureWriteError,
    StreamRead,
    StreamReader,
    StreamResult,
    StreamWrite,
    StreamWriter,
    AbiBuffer,
    backpressure_dec,
    backpressure_inc,
    yield_async,
    yield_blocking,
};
