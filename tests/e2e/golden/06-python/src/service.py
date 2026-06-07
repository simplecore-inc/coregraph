from .models import User, UserRepository


class UserService:
    def __init__(self, repo: UserRepository) -> None:
        self._repo = repo

    def register(self, user_id: str, name: str) -> User:
        user = User(user_id, name)
        self._repo.save(user)
        return user

    def lookup(self, user_id: str) -> User:
        user = self._repo.find(user_id)
        if user is None:
            raise KeyError(user_id)
        return user


def make_service() -> UserService:
    return UserService(UserRepository())
