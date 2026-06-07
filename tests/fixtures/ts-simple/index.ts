export class UserController {
    private userService: UserService;

    constructor(userService: UserService) {
        this.userService = userService;
    }

    async getUser(id: string): Promise<User> {
        return this.userService.findById(id);
    }
}

export interface UserService {
    findById(id: string): Promise<User>;
    findAll(): Promise<User[]>;
}

export type User = {
    id: string;
    name: string;
    email: string;
};

export function createController(svc: UserService): UserController {
    return new UserController(svc);
}
