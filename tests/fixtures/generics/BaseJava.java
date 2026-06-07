package com.example.generics;

// Fixture for Java inheritance (Inherits edge) and enum cross-match
// (EnumValueMatch via StringMatch reclassification).
public class BaseJava {
    public String name;
}

public class DerivedJava extends BaseJava {
    public int age;
}

public enum StatusJ {
    ACTIVE,
    INACTIVE
}

public enum StatusK {
    ACTIVE,
    PENDING
}
