class DataProcessor:
    def __init__(self, config):
        self.config = config

    def process(self, data: list) -> list:
        return [self._transform(item) for item in data]

    def _transform(self, item):
        return item

def create_processor(config):
    return DataProcessor(config)

MAX_BATCH_SIZE = 1000
