package main

import "fmt"

type Handler struct {
    Name string
}

func NewHandler(name string) *Handler {
    return &Handler{Name: name}
}

func (h *Handler) Serve() error {
    fmt.Println("serving:", h.Name)
    return nil
}

func main() {
    h := NewHandler("default")
    _ = h.Serve()
}
