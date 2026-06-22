import { Test, TestingModule } from '@nestjs/testing';
import { getRepositoryToken } from '@nestjs/typeorm';
import { Repository } from 'typeorm';
import { ConsumerManagementService } from './consumer-management.service';
import { WebhookConsumer } from '../entities/webhook-consumer.entity';
import { CreateConsumerDto } from '../dto/create-consumer.dto';

describe('ConsumerManagementService', () => {
  let service: ConsumerManagementService;
  let repository: Repository<WebhookConsumer>;

  const mockQueryBuilder = {
    where: jest.fn().mockReturnThis(),
    orderBy: jest.fn().mockReturnThis(),
    getMany: jest.fn(),
  };

  const mockConsumerRepository = {
    create: jest.fn(),
    save: jest.fn(),
    findOne: jest.fn(),
    find: jest.fn(),
    remove: jest.fn(),
    count: jest.fn(),
    createQueryBuilder: jest.fn(() => mockQueryBuilder),
  };

  beforeEach(async () => {
    const module: TestingModule = await Test.createTestingModule({
      providers: [
        ConsumerManagementService,
        {
          provide: getRepositoryToken(WebhookConsumer),
          useValue: mockConsumerRepository,
        },
      ],
    }).compile();

    service = module.get<ConsumerManagementService>(ConsumerManagementService);
    repository = module.get<Repository<WebhookConsumer>>(
      getRepositoryToken(WebhookConsumer),
    );
  });

  it('should be defined', () => {
    expect(service).toBeDefined();
  });

  describe('createConsumer', () => {
    it('should create a new consumer', async () => {
      const createDto: CreateConsumerDto = {
        name: 'Test Consumer',
        url: 'https://example.com/webhook',
        maxRetries: 3,
        timeoutMs: 5000,
      };

      const consumer = new WebhookConsumer();
      Object.assign(consumer, createDto, { id: 'test-id', isActive: true });

      mockConsumerRepository.findOne.mockResolvedValue(null);
      mockConsumerRepository.create.mockReturnValue(consumer);
      mockConsumerRepository.save.mockResolvedValue(consumer);

      const result = await service.createConsumer(createDto);

      expect(result).toEqual(consumer);
      expect(mockConsumerRepository.findOne).toHaveBeenCalledWith({
        where: { url: createDto.url },
      });
      expect(mockConsumerRepository.create).toHaveBeenCalledWith({
        ...createDto,
        status: 'active',
      });
      expect(mockConsumerRepository.save).toHaveBeenCalledWith(consumer);
    });
  });

  describe('getAllConsumers', () => {
    it('should return all consumers', async () => {
      const consumers = [new WebhookConsumer(), new WebhookConsumer()];
      mockQueryBuilder.getMany.mockResolvedValue(consumers);

      const result = await service.getAllConsumers();

      expect(result).toEqual(consumers);
      expect(mockConsumerRepository.createQueryBuilder).toHaveBeenCalled();
      expect(mockQueryBuilder.getMany).toHaveBeenCalled();
    });
  });
});
