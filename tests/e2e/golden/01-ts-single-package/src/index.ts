import { UserController } from "./controller";
import { makeUserService, UserRepository } from "./user";

export function bootstrap(repo: UserRepository): UserController {
    const service = makeUserService(repo);
    return new UserController(service);
}
