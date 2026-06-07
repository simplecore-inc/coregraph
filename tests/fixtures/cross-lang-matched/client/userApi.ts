export async function listUsers(): Promise<unknown[]> {
    const res = await fetch("/api/v1/users");
    return res.json();
}

export async function createUser(body: unknown): Promise<unknown> {
    const res = await fetch("/api/v1/users", {
        method: "POST",
        body: JSON.stringify(body),
    });
    return res.json();
}
