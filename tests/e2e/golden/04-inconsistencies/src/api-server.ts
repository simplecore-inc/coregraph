const routes = {
    users: "/api/v1/users",
    orders: "/api/v1/orders",
    cards: "/api/v1/cards",
};

export function serverRoute(key: keyof typeof routes): string {
    return routes[key];
}
