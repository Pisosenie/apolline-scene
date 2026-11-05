// What SceneScript sees: vectors, the engine, the scene's objects and the loop that runs each
// script. Every object's own state (before its parents move it) lives in one array the host
// reads back after each tick.
'use strict';
(() => {
const STRIDE = 15;
const ORIGIN = 0, SCALE = 3, ANGLES = 6, COLOR = 9, ALPHA = 12, BRIGHTNESS = 13, VISIBLE = 14;

// Members scripts asked for that don't exist, to name in a failure.
const asked = new Set();
const watched = (target, label) => new Proxy(target, {
    get(inner, key, receiver) {
        if (typeof key === 'string' && !(key in inner)) {
            asked.add(`${label}.${key}`);
        }
        return Reflect.get(inner, key, receiver);
    },
});
// What runs now, so a frame cut short can blame it.
let running = null;
function guarded(fail, run) {
    running = fail;
    asked.clear();
    try {
        run();
    } catch (error) {
        const message = String(error && error.message !== undefined ? error.message : error);
        fail(asked.size > 0 ? `${message} (it asked for ${[...asked].join(', ')})` : message);
    }
    running = null;
}

const finite = (n, fallback) => (typeof n === 'number' && isFinite(n) ? n : fallback);

function vectorClass(keys) {
    class Vector {
        constructor(...given) {
            const first = given[0];
            let parts;
            if (typeof first === 'string') {
                parts = first.trim().split(/\s+/).map(Number);
            } else if (first !== null && typeof first === 'object') {
                parts = keys.map((key) => first[key]);
            } else if (given.length === 1) {
                parts = keys.map(() => first);
            } else {
                parts = given;
            }
            keys.forEach((key, at) => {
                this[key] = finite(parts[at], 0);
            });
        }
        static fromString(text) {
            return new Vector(String(text));
        }
        zip(other, op) {
            const out = new Vector();
            for (const key of keys) {
                out[key] = op(this[key], typeof other === 'number' ? other : finite(other[key], 0));
            }
            return out;
        }
        map(op) {
            return this.zip(0, op);
        }
        add(other) { return this.zip(other, (a, b) => a + b); }
        subtract(other) { return this.zip(other, (a, b) => a - b); }
        multiply(other) { return this.zip(other, (a, b) => a * b); }
        divide(other) { return this.zip(other, (a, b) => a / b); }
        min(other) { return this.zip(other, Math.min); }
        max(other) { return this.zip(other, Math.max); }
        mix(other, t) { return this.zip(other, (a, b) => a + (b - a) * t); }
        abs() { return this.map(Math.abs); }
        floor() { return this.map(Math.floor); }
        ceil() { return this.map(Math.ceil); }
        round() { return this.map(Math.round); }
        negate() { return this.map((a) => -a); }
        dot(other) { return keys.reduce((sum, key) => sum + this[key] * other[key], 0); }
        lengthSqr() { return this.dot(this); }
        length() { return Math.sqrt(this.lengthSqr()); }
        distance(other) { return this.subtract(other).length(); }
        normalize() {
            const length = this.length();
            return length > 0 ? this.divide(length) : this.copy();
        }
        reflect(normal) { return this.subtract(new Vector(normal).multiply(2 * this.dot(normal))); }
        copy() { return new Vector(this); }
        equals(other) { return keys.every((key) => this[key] === other[key]); }
        toString() { return keys.map((key) => this[key]).join(' '); }
    }
    return Vector;
}
const Vec2 = vectorClass(['x', 'y']);
const Vec3 = vectorClass(['x', 'y', 'z']);
const Vec4 = vectorClass(['x', 'y', 'z', 'w']);
Vec2.prototype.perpendicular = function () { return new Vec2(-this.y, this.x); };
Vec3.prototype.cross = function (o) {
    return new Vec3(this.y * o.z - this.z * o.y, this.z * o.x - this.x * o.z, this.x * o.y - this.y * o.x);
};

let state = new Float64Array(0);
let names = [];
let parents = [];
let sizes = [];
const layers = [];
const scripts = [];
let timers = [];
const buffers = [];
const timerFailures = [];
let nextTimer = 1;
let canvas = new Vec2(1920, 1080);
let screen = new Vec2(1920, 1080);

const vector3 = (index, at) => new Vec3(state[index * STRIDE + at], state[index * STRIDE + at + 1], state[index * STRIDE + at + 2]);
function store3(index, at, value) {
    if (value === null || typeof value !== 'object') {
        return;
    }
    const base = index * STRIDE + at;
    ['x', 'y', 'z'].forEach((key, offset) => {
        state[base + offset] = finite(value[key], state[base + offset]);
    });
}
const store1 = (index, at, value) => {
    state[index * STRIDE + at] = finite(typeof value === 'boolean' ? Number(value) : value, state[index * STRIDE + at]);
};

class Layer {
    constructor(index) { this.index = index; }
    get name() { return names[this.index]; }
    get origin() { return vector3(this.index, ORIGIN); }
    set origin(value) { store3(this.index, ORIGIN, value); }
    get scale() { return vector3(this.index, SCALE); }
    set scale(value) { store3(this.index, SCALE, value); }
    get angles() { return vector3(this.index, ANGLES); }
    set angles(value) { store3(this.index, ANGLES, value); }
    get color() { return vector3(this.index, COLOR); }
    set color(value) { store3(this.index, COLOR, value); }
    get alpha() { return state[this.index * STRIDE + ALPHA]; }
    set alpha(value) { store1(this.index, ALPHA, value); }
    get brightness() { return state[this.index * STRIDE + BRIGHTNESS]; }
    set brightness(value) { store1(this.index, BRIGHTNESS, value); }
    get visible() { return state[this.index * STRIDE + VISIBLE] !== 0; }
    set visible(value) { store1(this.index, VISIBLE, Boolean(value)); }
    get size() { return new Vec2(sizes[2 * this.index], sizes[2 * this.index + 1]); }
    getParent() { return parents[this.index] < 0 ? null : layers[parents[this.index]]; }
}

const slots = { origin: ORIGIN, scale: SCALE, angles: ANGLES, color: COLOR, alpha: ALPHA, brightness: BRIGHTNESS, visible: VISIBLE };
function read(script) {
    const at = slots[script.property];
    if (at === VISIBLE) {
        return layers[script.object].visible;
    }
    return at === ALPHA || at === BRIGHTNESS ? state[script.object * STRIDE + at] : vector3(script.object, at);
}
function write(script, value) {
    const layer = layers[script.object];
    layer[script.property] = value;
}

function spectrumInto(buffer, spectrum) {
    const size = buffer.left.length;
    const span = 64 / size;
    for (let bin = 0; bin < size; bin++) {
        let left = 0, right = 0;
        for (let at = bin * span; at < (bin + 1) * span; at++) {
            left += spectrum ? spectrum[at] : 0;
            right += spectrum ? spectrum[64 + at] : 0;
        }
        buffer.left[bin] = left / span;
        buffer.right[bin] = right / span;
        buffer.average[bin] = (left + right) / (2 * span);
    }
}

function schedule(callback, delay, repeat) {
    if (typeof callback !== 'function') {
        throw new TypeError('a timer needs a function');
    }
    const timer = { id: nextTimer++, callback, every: Math.max(finite(delay, 0), 1) / 1000, repeat, due: engine.runtime + Math.max(finite(delay, 0), 0) / 1000 };
    timers.push(timer);
    const handle = { id: timer.id, stop() { timer.stopped = true; } };
    return handle;
}
function cancel(handle) {
    const id = handle !== null && typeof handle === 'object' ? handle.id : handle;
    for (const timer of timers) {
        if (timer.id === id) {
            timer.stopped = true;
        }
    }
}

const engine = watched({
    runtime: 0,
    frametime: 0,
    timeOfDay: 0,
    userProperties: {},
    AUDIO_RESOLUTION_16: 16,
    AUDIO_RESOLUTION_32: 32,
    AUDIO_RESOLUTION_64: 64,
    get canvasSize() { return canvas.copy(); },
    get screenResolution() { return screen.copy(); },
    registerAudioBuffers(resolution) {
        const size = [16, 32, 64].includes(resolution) ? resolution : 16;
        const buffer = { left: new Float32Array(size), right: new Float32Array(size), average: new Float32Array(size) };
        buffers.push(buffer);
        return buffer;
    },
    setTimeout: (callback, delay) => schedule(callback, delay, false),
    setInterval: (callback, delay) => schedule(callback, delay, true),
    clearTimeout: cancel,
    clearInterval: cancel,
    isDesktopDevice: () => true,
    isMobileDevice: () => false,
    isWallpaper: () => true,
    isScreensaver: () => false,
    isRunningInEditor: () => false,
}, 'engine');

const thisScene = watched({
    getLayer(name) {
        return layers.find((layer) => layer.name === name) ?? null;
    },
    getLayerCount: () => layers.length,
    getLayerIndex: (layer) => layers.indexOf(layer),
}, 'thisScene');

const input = watched({
    get cursorWorldPosition() { return new Vec3(canvas.x / 2, canvas.y / 2, 0); },
    get cursorScreenPosition() { return new Vec2(screen.x / 2, screen.y / 2); },
    cursorLeftDown: false,
}, 'input');

const stored = new Map();
const localStorage = {
    get: (key) => (stored.has(key) ? stored.get(key) : undefined),
    set: (key, value) => { stored.set(key, value); },
    remove: (key) => { stored.delete(key); },
    delete: (key) => { stored.delete(key); },
    clear: () => { stored.clear(); },
};

const quiet = () => {};
Object.assign(globalThis, {
    Vec2, Vec3, Vec4, engine, input, thisScene, localStorage,
    shared: {},
    console: { log: quiet, info: quiet, warn: quiet, error: quiet, debug: quiet },
    setTimeout: engine.setTimeout,
    setInterval: engine.setInterval,
    clearTimeout: cancel,
    clearInterval: cancel,
    MediaPlaybackEvent: { PLAYBACK_STOPPED: 0, PLAYBACK_PLAYING: 1, PLAYBACK_PAUSED: 2 },
});

// A script property is a plain value, or bound to a user property; a colour is a vector.
function property(kind, value) {
    const plain = value !== null && typeof value === 'object' && 'value' in value ? value.value : value;
    return kind === 'addColor' && typeof plain === 'string' ? new Vec3(plain) : plain;
}

// Scripts can't replace what the host calls.
Object.defineProperty(globalThis, '__world', { writable: false, configurable: false, value: Object.freeze({
    // Objects as [name, parent index or -1, width, height] and their state, flattened.
    setup({ objects, initial, width, height, user }) {
        names = objects.map((object) => object[0]);
        parents = objects.map((object) => object[1]);
        sizes = objects.flatMap((object) => [object[2], object[3]]);
        state = Float64Array.from(initial);
        objects.forEach((_, index) => layers.push(watched(new Layer(index), 'layer')));
        canvas = new Vec2(width, height);
        engine.userProperties = user;
    },
    layer: (index) => layers[index],
    // The builder `createScriptProperties()` returns, with the scene's own values over the
    // script's defaults.
    properties(overrides) {
        const values = {};
        const builder = new Proxy({}, {
            get(_, key) {
                if (key === 'finish') {
                    return () => values;
                }
                if (typeof key === 'string' && key.startsWith('add')) {
                    return (definition) => {
                        if (definition && definition.name !== undefined) {
                            const given = Object.prototype.hasOwnProperty.call(overrides, definition.name);
                            values[definition.name] = property(key, given ? overrides[definition.name] : definition.value);
                        }
                        return builder;
                    };
                }
                return undefined;
            },
        });
        return builder;
    },
    add(object, property, exports) {
        scripts.push({ object, property, init: exports.init, update: exports.update, started: false, error: null });
    },
    tick(runtime, frametime, daytime, width, height, spectrum) {
        engine.runtime = runtime;
        engine.frametime = frametime;
        engine.timeOfDay = daytime;
        screen = new Vec2(width, height);
        for (const buffer of buffers) {
            spectrumInto(buffer, spectrum);
        }
        // The queue stays whole while callbacks run, so a frame cut short loses none of it.
        for (const timer of timers.slice()) {
            if (timer.stopped || timer.due > runtime) {
                continue;
            }
            if (timer.repeat) {
                timer.due = Math.max(timer.due + timer.every, runtime);
            } else {
                timer.stopped = true;
            }
            guarded((message) => {
                timer.stopped = true;
                if (timerFailures.length < 16) {
                    timerFailures.push(message);
                }
            }, timer.callback);
        }
        timers = timers.filter((timer) => !timer.stopped);
        for (const script of scripts.filter((script) => script.error === null)) {
            guarded((message) => { script.error = message; }, () => {
                if (!script.started) {
                    script.started = true;
                    if (typeof script.init === 'function') {
                        const value = script.init(read(script));
                        if (value !== undefined) {
                            write(script, value);
                        }
                    }
                }
                if (typeof script.update === 'function') {
                    const value = script.update(read(script));
                    if (value !== undefined) {
                        write(script, value);
                    }
                }
            });
        }
        return Array.from(state).concat([buffers.length > 0 ? 1 : 0]);
    },
    // Scripts that threw, as [object index, property, message]; they don't run again.
    failures: () => scripts
        .filter((script) => script.error !== null)
        .map((script) => [String(script.object), script.property, script.error])
        .concat(timerFailures.map((message) => ['-1', 'timer', message])),
    // A frame cut short leaves what was running then; it doesn't run again. Whether there
    // was anything to blame.
    abandon() {
        const blamed = running !== null;
        if (blamed) {
            running('it ran too long');
            running = null;
        }
        return blamed;
    },
    // The members scripts asked for that aren't here, since this was last asked.
    asking() {
        const text = [...asked].join(', ');
        asked.clear();
        return text;
    },
}) });
})();
