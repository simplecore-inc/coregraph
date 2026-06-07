export interface Box<T> {
    value: T;
}

export class Holder<T> {
    constructor(private item: T) {}
    get(): T {
        return this.item;
    }
}

export function unwrap<T>(b: Box<T>): T {
    return b.value;
}
