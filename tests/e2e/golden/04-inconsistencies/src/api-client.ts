export const API_USERS = "/api/v1/user";
export const API_ORDERS = "/api/v1/orders";
export const API_CARDS = "/api/v1/card";

export async function fetchUsers(): Promise<unknown> {
    return fetch(API_USERS);
}
