from typing import Optional


class User:
    def __init__(self, user_id: str, name: str) -> None:
        self.user_id = user_id
        self.name = name

    def display(self) -> str:
        return f"{self.name} ({self.user_id})"


class UserRepository:
    def __init__(self) -> None:
        self._users: dict[str, User] = {}

    def save(self, user: User) -> None:
        self._users[user.user_id] = user

    def find(self, user_id: str) -> Optional[User]:
        return self._users.get(user_id)
