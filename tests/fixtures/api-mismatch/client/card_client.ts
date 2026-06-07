// Client forgets the trailing 's': "/api/v1/card" instead of "/api/v1/cards".
// CoreGraph should flag this as an api-path inconsistency
// (Levenshtein distance = 1, length diff ≤ 20%).
export async function loadCards() {
    const res = await fetch("/api/v1/card");
    return res.json();
}

export async function createCard(body: unknown) {
    const res = await fetch("/api/v1/card", {
        method: "POST",
        body: JSON.stringify(body),
    });
    return res.json();
}
