// Main-thread worker factory. Every Rust thread, including the root worker,
// asks here for new threads through a MessagePort, so workers are only ever
// created on the main thread, which never blocks.

export class ThreadFactory {
    constructor(module) {
        this.module = module;
        this.memory = null;
        this.pending = [];
        this.workers = [];
        this.onError = () => {};
    }

    /// A port to hand to a worker for its spawn requests.
    port() {
        const channel = new MessageChannel();
        channel.port1.onmessage = ({ data }) => this.spawn(data);
        return channel.port2;
    }

    /// Called once the root worker has created the shared memory.
    setMemory(memory) {
        this.memory = memory;
        for (const request of this.pending.splice(0)) {
            this.spawn(request);
        }
    }

    /// Stops every thread and refuses further spawns.
    terminateAll() {
        this.memory = null;
        this.spawn = () => {};
        for (const worker of this.workers.splice(0)) {
            worker.terminate();
        }
    }

    spawn(request) {
        if (!this.memory) {
            this.pending.push(request);
            return;
        }
        const worker = new Worker(new URL("./thread-worker.js?v=__BUILD__", import.meta.url), {
            type: "module",
            name: request.name,
        });
        worker.onerror = (event) => this.onError(`${request.name}: ${event.message}`);
        this.workers.push(worker);
        const port = this.port();
        worker.postMessage(
            {
                module: this.module,
                memory: this.memory,
                start: request.start,
                stackSize: request.stackSize,
                port,
            },
            [port],
        );
    }
}
