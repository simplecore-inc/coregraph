export class Widget {
    label(): string { return "widget"; }
}

export function spawn(): Widget {
    return new Widget();
}
