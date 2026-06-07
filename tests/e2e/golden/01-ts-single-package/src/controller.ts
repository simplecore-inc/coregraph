import { UserService, User, UserId } from "./user";

export class UserController {
    constructor(private service: UserService) {}

    async handleGet(id: UserId): Promise<User> {
        return this.service.getUser(id);
    }

    async handleList(): Promise<User[]> {
        return this.service.listUsers();
    }

    async handleCreate(user: User): Promise<void> {
        return this.service.createUser(user);
    }
}
