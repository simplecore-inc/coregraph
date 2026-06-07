package com.example;

public class HelloWorld {
    private String message;

    public HelloWorld(String message) {
        this.message = message;
    }

    public String greet() {
        return "Hello, " + message;
    }
}
