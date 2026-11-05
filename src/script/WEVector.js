// Angles in degrees.
export function angleVector2(angle) {
    const radians = angle * Math.PI / 180;
    return new Vec2(Math.cos(radians), Math.sin(radians));
}
export function vectorAngle2(vector) {
    return Math.atan2(vector.y, vector.x) * 180 / Math.PI;
}
