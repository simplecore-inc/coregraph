package com.example

class MyService {
    fun processData(input: String): String {
        return input.trim()
    }
}

interface DataProcessor {
    fun process(data: String): String
}
