// Bootstrap for a spawned Rust thread. The main thread sends the compiled
// module, the shared memory, the thread's start record and a port for the
// thread's own spawn requests.
import init, { __vearth_attach_spawner, __vearth_run_thread } from "./pkg/vearth_web.js?v=__BUILD__";

self.onmessage = async ({ data }) => {
    const { module, memory, start, stackSize, port } = data;
    const wasm = await init({ module_or_path: module, memory, thread_stack_size: stackSize });
    __vearth_attach_spawner(port);
    const persistent = __vearth_run_thread(start);
    if (!persistent) {
        wasm.__wbindgen_thread_destroy();
        close();
    }
};
