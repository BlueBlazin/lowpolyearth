// Main thread: checks the browser, starts the root worker on an
// OffscreenCanvas, creates worker threads on request, forwards DOM input and
// performs the page-level requests the app posts back. No app code runs here.
import { ThreadFactory } from "./threads.js?v=__BUILD__";

const canvas = document.getElementById("app");
const status = document.getElementById("status");
// Receives typing while an app text field is being edited, as in eframe:
// only an editable element gets input-method composition and inserted text.
// Placed at egui's caret so candidate windows appear beside the text. At all
// other times the canvas holds focus, so holding a flight key does not open
// the system's accent picker, which appears for editable elements.
const textAgent = document.getElementById("text-agent");
let editingText = false;
const focusInput = () =>
    (editingText ? textAgent : canvas).focus({ preventScroll: true });

// Fades out and removes the collage shown while the app loads.
function hideCollage() {
    const collage = document.getElementById("collage");
    collage?.classList.add("done");
    setTimeout(() => collage?.remove(), 1200);
}

// Messages keep the collage behind them; only a ready app replaces it.
function showError(message, controls = "") {
    status.className = "error";
    status.innerHTML = `<div>${message}</div>${controls}`;
}

const MAC_APP = '<a href="https://lowpolyearth.online/">Low Poly Earth for Mac</a>';
const CHROME = '<a href="https://www.google.com/chrome/">Chrome</a>';
// Other browsers with the required features may run the app; releases are
// tested in Chrome, so failures elsewhere say so.
const chromium = navigator.userAgentData?.brands.some(({ brand }) => brand === "Chromium") ?? false;
const testedIn = chromium ? "" : ` Low Poly Earth is tested in desktop ${CHROME}.`;

function showFailure(message) {
    showError(`${message} <a href="">Reload</a>.${testedIn}`);
}

function unsupported() {
    const agent = navigator.userAgentData;
    // iPadOS presents itself as a Mac; touch support gives it away.
    const mobile = agent
        ? agent.mobile
        : /Android|iPhone|iPad|iPod/.test(navigator.userAgent) ||
          (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
    if (mobile) {
        return "<strong>Low Poly Earth needs a desktop computer</strong>" +
            "Open this page on a Mac or PC, or get " + MAC_APP + ".";
    }
    if (!crossOriginIsolated) {
        return "<strong>Low Poly Earth needs shared memory</strong>" +
            "This page was served without cross-origin isolation, so the browser " +
            "disabled SharedArrayBuffer." + testedIn;
    }
    if (!navigator.gpu) {
        return "<strong>Low Poly Earth needs WebGPU</strong>" +
            "This browser has WebGPU unavailable or turned off. Check that it is " +
            "current and that hardware acceleration is on." + testedIn;
    }
    if (!("transferControlToOffscreen" in HTMLCanvasElement.prototype)) {
        return "<strong>Low Poly Earth needs OffscreenCanvas</strong>" +
            "This browser cannot render to a canvas from a worker." + testedIn;
    }
    return null;
}

// The app's render-resolution preference caps the device pixel ratio the
// canvas renders at; the browser scales the canvas to its CSS size.
let maxPixelRatio = Infinity;
const pixelRatio = () => Math.min(window.devicePixelRatio || 1, maxPixelRatio);

// `box` is the observed device-pixel content box, which is exact where
// multiplying the CSS size by the pixel ratio can round differently.
function canvasSize(box) {
    const rect = canvas.getBoundingClientRect();
    const scale = pixelRatio();
    const deviceToRender = scale / (window.devicePixelRatio || 1);
    const [width, height] = box
        ? [box.inlineSize * deviceToRender, box.blockSize * deviceToRender]
        : [rect.width * scale, rect.height * scale];
    return {
        width: Math.max(1, Math.round(width)),
        height: Math.max(1, Math.round(height)),
        scale,
    };
}

// Checked before the app downloads. A software adapter renders the app far
// too slowly to be usable. `navigator.deviceMemory` is coarse (Chrome caps it
// at 8), so it only identifies machines with clearly too little memory.
async function hardwareProblem() {
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
    if (!adapter) {
        return {
            block: "<strong>No graphics adapter is available</strong>" +
                "The browser offers WebGPU but found no GPU to use. Check that " +
                "hardware acceleration is on." + testedIn,
        };
    }
    if (adapter.info?.isFallbackAdapter ?? adapter.isFallbackAdapter) {
        return {
            block: "<strong>This browser is drawing without a GPU</strong>" +
                "It offers only a software renderer, which is far too slow for " +
                "Low Poly Earth. Turn on hardware acceleration, or try another " +
                "computer.",
        };
    }
    const memory = navigator.deviceMemory;
    if (memory && memory < 8) {
        return {
            warn: "<strong>This computer may be too weak</strong>" +
                `It reports ${memory} GB of memory. Low Poly Earth uses several ` +
                "gigabytes, so it may run slowly or be closed by the browser.",
        };
    }
    return {};
}

const WEAK_ACCEPTED = "vearth.weakMachineAccepted";

function acceptWarning(message) {
    try {
        if (localStorage.getItem(WEAK_ACCEPTED)) {
            return Promise.resolve();
        }
    } catch {}
    showError(message, '<button id="continue" type="button">Continue anyway</button>');
    return new Promise((resolve) => {
        document.getElementById("continue").addEventListener("click", () => {
            try {
                localStorage.setItem(WEAK_ACCEPTED, "1");
            } catch {}
            status.className = "";
            status.textContent = "Loading Low Poly Earth…";
            resolve();
        });
    });
}

async function run() {
    const problem = unsupported();
    if (problem) {
        showError(problem);
        return;
    }
    const hardware = await hardwareProblem();
    if (hardware.block) {
        showError(hardware.block);
        return;
    }
    if (hardware.warn) {
        await acceptWarning(hardware.warn);
    }
    const [module, manifest] = await Promise.all([
        WebAssembly.compileStreaming(fetch("./pkg/vearth_web_bg.wasm?v=__BUILD__")),
        fetch("./asset-manifest.json?v=__BUILD__").then((response) => response.text()),
    ]);
    const threads = new ThreadFactory(module);
    // Console diagnostics: the shared WebAssembly memory's current size.
    globalThis.vearthMemoryMiB = () => (threads.memory?.buffer.byteLength ?? 0) / 2 ** 20;
    threads.onError = (message) => console.error(`worker error: ${message}`);

    const root = new Worker(new URL("./app-worker.js?v=__BUILD__", import.meta.url), {
        type: "module",
        name: "vearth-main",
    });
    const send = (event) => root.postMessage({ type: "input", event });
    root.onerror = (event) =>
        showFailure(`Low Poly Earth stopped: ${event.message}.`);
    root.onmessage = ({ data }) => {
        if (data.type === "clearSiteData") {
            root.terminate();
            threads.terminateAll();
            clearSiteData();
        } else {
            pageRequest(data, threads, send);
        }
    };

    const size = canvasSize();
    const offscreen = canvas.transferControlToOffscreen();
    const port = threads.port();
    root.postMessage(
        {
            type: "start",
            mode: "app",
            module,
            port,
            canvas: offscreen,
            manifest,
            assetBase: new URL("./assets/", location.href).href,
            query: location.search,
            ...size,
        },
        [port, offscreen],
    );
    forwardInput(send);
    focusInput();
}

// Removes everything the site stored once the app has stopped: the HTTP
// cache for this origin (map ranges and app files) through the server's
// Clear-Site-Data response, and site storage (saved terrain, settings) both
// through that response and directly, since browsers apply the header's
// storage clearing asynchronously. Browsers do not let a site clear what it
// cached from other origins, so elevation tiles stay in the HTTP cache.
async function clearSiteData() {
    canvas.style.display = "none";
    status.className = "busy";
    status.innerHTML =
        '<div class="spinner"></div><strong>Removing data</strong>Low Poly Earth has stopped.';
    // Keep the progress visible long enough to read, even when clearing is quick.
    const shown = new Promise((resolve) => setTimeout(resolve, 800));
    try {
        const response = await fetch("./clear-site-data", { method: "POST", cache: "no-store" });
        if (!response.ok) {
            throw new Error(`the server answered ${response.status}`);
        }
        localStorage.clear();
        sessionStorage.clear();
        const files = await navigator.storage.getDirectory();
        for await (const name of files.keys()) {
            await files.removeEntry(name, { recursive: true });
        }
        await shown;
        showError(
            "<strong>All data removed</strong>Saved terrain, map data and settings were " +
                "removed from this browser. Elevation tiles stay in the browser's cache " +
                "until it clears them.",
            '<button type="button">Start again</button>',
        );
        status.querySelector("button").addEventListener("click", () => location.reload());
    } catch (error) {
        showFailure(`Some data could not be removed: ${error.message}`);
    }
}

function pageRequest(message, threads, send) {
    const { type, value } = message;
    switch (type) {
        case "memory":
            threads.setMemory(message.memory);
            break;
        case "ready":
            status.className = "hidden";
            hideCollage();
            break;
        case "fatal":
            console.error(value);
            showFailure(`Low Poly Earth stopped: ${value}.`);
            break;
        case "maxPixelRatio": {
            const next = value > 0 ? value : Infinity;
            if (next !== maxPixelRatio) {
                maxPixelRatio = next;
                send({ kind: "resize", ...canvasSize() });
            }
            break;
        }
        case "title":
            document.title = value;
            break;
        case "cursorVisible":
            canvas.style.cursor = value ? "" : "none";
            break;
        case "imeCursor":
            if (value) {
                const [x, y, height] = value;
                Object.assign(textAgent.style, { left: `${x}px`, top: `${y}px`, height: `${height}px` });
            }
            if (Boolean(value) !== editingText) {
                editingText = Boolean(value);
                focusInput();
            }
            break;
        case "cursorIcon":
            canvas.dataset.cursor = value;
            if (canvas.style.cursor !== "none") {
                canvas.style.cursor = value;
            }
            break;
        case "openUrl":
            window.open(value, "_blank", "noopener");
            break;
        case "copyText":
            navigator.clipboard?.writeText(value).catch((error) => console.warn(error));
            break;
        case "download": {
            const url = URL.createObjectURL(new Blob([value.bytes], { type: value.mime }));
            const link = Object.assign(document.createElement("a"), {
                href: url,
                download: value.name,
            });
            link.click();
            setTimeout(() => URL.revokeObjectURL(url), 60_000);
            break;
        }
    }
}

// Keys whose browser default would move focus away from the page.
const PREVENT_KEYS = new Set(["Tab"]);
const PREVENT_SHORTCUTS = new Set(["KeyO", "KeyP", "KeyS", "Comma"]);

function forwardInput(send) {
    const position = (event) => ({
        x: event.offsetX * pixelRatio(),
        y: event.offsetY * pixelRatio(),
    });

    new ResizeObserver((entries) => {
        const entry = entries[entries.length - 1];
        send({ kind: "resize", ...canvasSize(entry.devicePixelContentBoxSize?.[0]) });
    }).observe(canvas, { box: "device-pixel-content-box" });

    canvas.addEventListener("pointermove", (event) => {
        send({ kind: "pointermove", ...position(event) });
    });
    // Tilting is Ctrl/Cmd-drag and flying forward is W, but Ctrl/Cmd+W closes
    // the tab and pages cannot intercept it. A close during a modifier drag
    // is almost certainly that accident, so only then does the page ask.
    let modifierDrag = false;
    window.addEventListener("beforeunload", (event) => {
        if (modifierDrag) {
            event.preventDefault();
        }
    });
    canvas.addEventListener("pointerup", () => (modifierDrag = false));
    canvas.addEventListener("pointercancel", () => (modifierDrag = false));
    canvas.addEventListener("pointerdown", (event) => {
        modifierDrag = event.ctrlKey || event.metaKey;
        focusInput();
        canvas.setPointerCapture(event.pointerId);
        send({ kind: "pointermove", ...position(event) });
        send({ kind: "pointerbutton", button: event.button, pressed: true });
        event.preventDefault();
    });
    canvas.addEventListener("pointerup", (event) => {
        send({ kind: "pointermove", ...position(event) });
        send({ kind: "pointerbutton", button: event.button, pressed: false });
    });
    canvas.addEventListener("pointerenter", () => send({ kind: "pointerinside", inside: true }));
    canvas.addEventListener("pointerleave", () => send({ kind: "pointerinside", inside: false }));
    canvas.addEventListener("contextmenu", (event) => event.preventDefault());
    canvas.addEventListener(
        "wheel",
        (event) => {
            event.preventDefault();
            const pixels = event.deltaMode === WheelEvent.DOM_DELTA_PIXEL;
            send({
                kind: "wheel",
                dx: pixels ? event.deltaX * pixelRatio() : event.deltaX,
                dy: pixels ? event.deltaY * pixelRatio() : event.deltaY,
                mode: event.deltaMode,
            });
        },
        { passive: false },
    );

    const key = (pressed) => (event) => {
        // Composition keystrokes arrive as text when the composition ends.
        if (event.isComposing || event.keyCode === 229) {
            return;
        }
        const shortcut = event.ctrlKey || event.metaKey;
        if (PREVENT_KEYS.has(event.code) || (shortcut && PREVENT_SHORTCUTS.has(event.code))) {
            event.preventDefault();
        }
        send({
            kind: "key",
            code: event.code,
            key: event.key,
            pressed,
            repeat: event.repeat,
            shift: event.shiftKey,
            control: event.ctrlKey,
            alt: event.altKey,
            meta: event.metaKey,
            fromTextAgent: event.target === textAgent,
        });
    };
    window.addEventListener("keydown", key(true));
    window.addEventListener("keyup", key(false));

    textAgent.addEventListener("input", (event) => {
        if (event.isComposing) {
            send({ kind: "composition", text: event.data ?? "", committed: false });
        } else if (textAgent.value) {
            send({ kind: "text", text: textAgent.value });
            textAgent.value = "";
        }
    });
    textAgent.addEventListener("compositionend", (event) => {
        send({ kind: "composition", text: event.data ?? "", committed: true });
        textAgent.value = "";
    });

    window.addEventListener("paste", (event) => {
        const text = event.clipboardData?.getData("text/plain");
        if (text) {
            send({ kind: "paste", text });
            event.preventDefault();
        }
    });
    window.addEventListener("copy", (event) => {
        send({ kind: "clipboard", cut: false });
        event.preventDefault();
    });
    window.addEventListener("cut", (event) => {
        send({ kind: "clipboard", cut: true });
        event.preventDefault();
    });

    window.addEventListener("focus", () => send({ kind: "focus", focused: true }));
    window.addEventListener("blur", () => send({ kind: "focus", focused: false }));
    document.addEventListener("visibilitychange", () =>
        send({ kind: "visibility", hidden: document.hidden }),
    );
    window.addEventListener("pagehide", () => send({ kind: "unload" }));
}

run().catch((error) => {
    console.error(error);
    showFailure(`Low Poly Earth could not start: ${error.message ?? error}.`);
});
