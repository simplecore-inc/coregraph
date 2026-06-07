package main

import "fmt"

type User struct {
	ID   int
	Name string
}

type UserRepository interface {
	FindByID(id int) (*User, error)
	FindAll() ([]*User, error)
}

func NewUser(id int, name string) *User {
	return &User{ID: id, Name: name}
}

func (u *User) String() string {
	return fmt.Sprintf("User(%d, %s)", u.ID, u.Name)
}
