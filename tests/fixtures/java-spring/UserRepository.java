package com.example;

public interface UserRepository {
    User findById(Long id);
    java.util.List<User> findAll();
}
