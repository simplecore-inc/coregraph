export function add(a: number, b: number): number {
    return a + b;
}

export function multiply(a: number, b: number): number {
    return a * b;
}

// Intentionally orphan: exported but never called.
export function divide(a: number, b: number): number {
    return a / b;
}

// Intentionally orphan: exported but never called.
export function modulo(a: number, b: number): number {
    return a % b;
}
