package com.example;

import java.util.List;

public interface UserService {
    List<String> findAll();
    String findById(long id);
}
