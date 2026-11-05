export const PI = Math.PI;
export const deg2rad = Math.PI / 180;
export const rad2deg = 180 / Math.PI;
export function mix(a, b, t) {
    return a + (b - a) * t;
}
export function clamp(value, low, high) {
    return Math.min(Math.max(value, low), high);
}
export function smoothStep(edge0, edge1, x) {
    const t = clamp((x - edge0) / (edge1 - edge0), 0, 1);
    return t * t * (3 - 2 * t);
}
