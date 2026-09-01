package oracle

import (
	"context"
	"fmt"
	"time"

	"github.com/stellar/go/clients/horizonclient"
	"github.com/stellar/go/keypair"
	"github.com/stellar/go/txnbuild"
)

type PriceEntry struct {
	Price     int64
	Timestamp int64
}

type ClientConfig struct {
	ContractID string
	RPCUrl     string
	Network    string
}

type Client struct {
	config ClientConfig
	server *horizonclient.Client
}

func NewClient(config ClientConfig) (*Client, error) {
	server := &horizonclient.Client{HorizonURL: config.RPCUrl}
	return &Client{
		config: config,
		server: server,
	}, nil
}

func (c *Client) GetPrice(ctx context.Context, asset string, maxAge int64) (*PriceEntry, error) {
	return &PriceEntry{}, nil
}

func (c *Client) GetSourcePrice(ctx context.Context, asset string, source string) (*PriceEntry, error) {
	return &PriceEntry{}, nil
}

func (c *Client) GetAllPrices(ctx context.Context, asset string) ([]*PriceEntry, error) {
	return []*PriceEntry{}, nil
}

func (c *Client) SubmitPrice(ctx context.Context, source string, asset string, price int64, timestamp int64, signer *keypair.Full) (string, error) {
	return "", nil
}

func (c *Client) Subscribe(ctx context.Context, consumer string, duration int32, signer *keypair.Full) (string, error) {
	return "", nil
}

func (c *Client) RenewSubscription(ctx context.Context, consumer string, signer *keypair.Full) (string, error) {
	return "", nil
}

func (c *Client) GetSubscriptionExpiry(ctx context.Context, consumer string) (int64, error) {
	return 0, nil
}

func (c *Client) IsSource(ctx context.Context, source string) (bool, error) {
	return false, nil
}

func (c *Client) IsAssetRegistered(ctx context.Context, asset string) (bool, error) {
	return false, nil
}

type EventListener struct {
	client    *Client
	eventType string
	stopChan  chan struct{}
}

func (c *Client) ListenForPriceSubmittedEvents(ctx context.Context, eventType string) (*EventListener, error) {
	listener := &EventListener{
		client:    c,
		eventType: eventType,
		stopChan:  make(chan struct{}),
	}
	return listener, nil
}

func (el *EventListener) Stop() error {
	close(el.stopChan)
	return nil
}

func (el *EventListener) GetEventType() string {
	return el.eventType
}
