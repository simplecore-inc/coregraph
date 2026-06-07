export enum Role {
    ADMIN = "admin",
    USER = "user",
    GUEST = "guest",
}

export function isAdmin(r: Role): boolean {
    return r === Role.ADMIN;
}
