import { add, multiply } from "./math";
import { greet } from "./greeter";

export function run(): void {
    const x = add(2, 3);
    const y = multiply(x, 4);
    greet(`result=${y}`);
}

run();
