import { Alpha, Beta } from "./inner";

export function demo(): string {
    return new Alpha().greet() + new Beta().greet();
}
