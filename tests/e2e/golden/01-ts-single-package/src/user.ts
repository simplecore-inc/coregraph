export type UserId = string;

export interface User {
    id: UserId;
    name: string;
    email: string;
}

export interface UserRepository {
    findById(id: UserId): Promise<User | null>;
    findAll(): Promise<User[]>;
    save(user: User): Promise<void>;
}

export class UserService {
    constructor(private repo: UserRepository) {}

    async getUser(id: UserId): Promise<User> {
        const user = await this.repo.findById(id);
        if (!user) throw new Error(`User ${id} not found`);
        return user;
    }

    async listUsers(): Promise<User[]> {
        return this.repo.findAll();
    }

    async createUser(user: User): Promise<void> {
        await this.repo.save(user);
    }
}

export function makeUserService(repo: UserRepository): UserService {
    return new UserService(repo);
}
