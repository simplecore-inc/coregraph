export async function loadPlugin(name: "a" | "b"): Promise<{ run(): string }> {
    const mod = await import("./plugin");
    return name === "a" ? new mod.PluginA() : new mod.PluginB();
}
