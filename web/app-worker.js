// Root worker: instantiates the module (creating the shared memory), reports
// the memory to the main thread's thread factory, then runs the app and
// dispatches the page's forwarded input to it.
import init, * as vearth from "./pkg/vearth_web.js?v=__BUILD__";

let started = false;
const queued = [];

function dispatch(event) {
    switch (event.kind) {
        case "resize":
            return vearth.resize(event.width, event.height, event.scale);
        case "pointermove":
            return vearth.pointer_move(event.x, event.y);
        case "pointerbutton":
            return vearth.pointer_button(event.button, event.pressed);
        case "pointerinside":
            return vearth.pointer_inside(event.inside);
        case "wheel":
            return vearth.wheel(event.dx, event.dy, event.mode);
        case "key":
            return vearth.key(
                event.code, event.key, event.pressed, event.repeat,
                event.shift, event.control, event.alt, event.meta, event.fromTextAgent,
            );
        case "text":
            return vearth.text(event.text);
        case "composition":
            return vearth.composition(event.text, event.committed);
        case "paste":
            return vearth.paste(event.text);
        case "clipboard":
            return vearth.clipboard_command(event.cut);
        case "focus":
            return vearth.focus(event.focused);
        case "visibility":
            return vearth.visibility(event.hidden);
        case "unload":
            return vearth.shutdown();
    }
}

function fatal(error) {
    postMessage({ type: "fatal", value: String(error?.stack ?? error) });
}

self.onerror = (event) => fatal(event.message);
self.onunhandledrejection = (event) => fatal(event.reason);

self.onmessage = async ({ data }) => {
    if (data.type === "input") {
        if (started) {
            try {
                dispatch(data.event);
            } catch (error) {
                fatal(error);
            }
        } else {
            queued.push(data.event);
        }
        return;
    }
    if (data.type !== "start") {
        return;
    }
    try {
        const wasm = await init({ module_or_path: data.module });
        vearth.init_runtime();
        vearth.__vearth_attach_spawner(data.port);
        postMessage({ type: "memory", memory: wasm.memory });
        if (data.mode === "selftest") {
            postMessage({ type: "selftest", report: vearth.run_platform_selftest() });
            return;
        }
        vearth.configure_site(data.assetBase, data.manifest);
        vearth.start(data.canvas, data.width, data.height, data.scale, data.query);
        started = true;
        for (const event of queued.splice(0)) {
            dispatch(event);
        }
    } catch (error) {
        fatal(error);
    }
};
