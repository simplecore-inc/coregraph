//go:build wireinject
// +build wireinject

package app

import "github.com/google/wire"

func NewStore() *Store { return &Store{} }

func NewService(s *Store) *Service { return &Service{store: s} }

func NewHandler(svc *Service) *Handler { return &Handler{svc: svc} }

func InitializeHandler() *Handler {
	wire.Build(NewStore, NewService, NewHandler)
	return &Handler{}
}

type Store struct{}
type Service struct{ store *Store }
type Handler struct{ svc *Service }
