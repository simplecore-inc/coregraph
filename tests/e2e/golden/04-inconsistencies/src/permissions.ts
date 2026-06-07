export enum Permission {
    ADMIN = "admin",
    READ = "read",
    WRITE = "write",
}

export function canWrite(p: Permission): boolean {
    return p === Permission.WRITE;
}
