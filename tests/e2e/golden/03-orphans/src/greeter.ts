export function greet(msg: string): void {
    console.log(msg);
}

// Intentionally orphan: exported but never imported.
export function farewell(msg: string): void {
    console.log(`goodbye: ${msg}`);
}
