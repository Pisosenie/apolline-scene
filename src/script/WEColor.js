// Hue, saturation and value each 0 to 1; the hue wraps around.
export function hsv2rgb(hsv) {
    const h = ((hsv.x % 1) + 1) % 1 * 6, s = hsv.y, v = hsv.z;
    const sector = Math.floor(h), f = h - sector;
    const p = v * (1 - s), q = v * (1 - s * f), t = v * (1 - s * (1 - f));
    const rgb = [[v, t, p], [q, v, p], [p, v, t], [p, q, v], [t, p, v], [v, p, q]][sector % 6];
    return new Vec3(rgb[0], rgb[1], rgb[2]);
}
export function rgb2hsv(rgb) {
    const high = Math.max(rgb.x, rgb.y, rgb.z), low = Math.min(rgb.x, rgb.y, rgb.z), span = high - low;
    let h = 0;
    if (span > 0) {
        if (high === rgb.x) h = ((rgb.y - rgb.z) / span + 6) % 6;
        else if (high === rgb.y) h = (rgb.z - rgb.x) / span + 2;
        else h = (rgb.x - rgb.y) / span + 4;
    }
    return new Vec3(h / 6, high > 0 ? span / high : 0, high);
}
export function normalizeColor(rgb) {
    return new Vec3(rgb).divide(255);
}
export function expandColor(rgb) {
    return new Vec3(rgb).multiply(255);
}
