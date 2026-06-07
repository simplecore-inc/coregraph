import os


def pool_size() -> int:
    return int(os.environ.get("DATABASE_POOL_SIZE", "5"))


def timeout_seconds() -> int:
    return int(os.environ.get("DATABASE_POOL_TIMEOUT", "30"))
