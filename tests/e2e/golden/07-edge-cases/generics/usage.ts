import { Box, Holder, unwrap } from "./container";

export function run(): number {
    const b: Box<number> = { value: 42 };
    const h = new Holder<string>("hello");
    return unwrap(b) + h.get().length;
}
