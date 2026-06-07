package com.example;

public class UserController {
    private final UserService service;

    public UserController(UserService service) {
        this.service = service;
    }

    public User handleGet(Long id) {
        return service.getUser(id);
    }
}
