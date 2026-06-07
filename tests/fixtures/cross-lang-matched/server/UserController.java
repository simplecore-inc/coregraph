package com.example.api;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class UserController {

    @GetMapping("/api/v1/users")
    public String listUsers() {
        return "[]";
    }

    @PostMapping("/api/v1/users")
    public String createUser() {
        return "{}";
    }
}
