import torch
from sentence_transformers import SentenceTransformer
import faiss
import numpy as np
from transformers import pipeline, AutoTokenizer, AutoModelForCausalLM
from typing import List, Tuple

class AdvancedRAG:
    def __init__(self, model_name: str = "gpt2"):
        # Initialize the sentence transformer model for embeddings
        self.embedding_model = SentenceTransformer('all-MiniLM-L6-v2')
        
        # Initialize FAISS index
        self.index = None
        self.documents = []
        
        # Initialize language model for generation
        self.tokenizer = AutoTokenizer.from_pretrained(model_name)
        self.generator = AutoModelForCausalLM.from_pretrained(model_name)
        
    def add_documents(self, documents: List[str]):
        """Add documents to the knowledge base"""
        # Create embeddings for documents
        embeddings = self.embedding_model.encode(documents)
        
        # Initialize FAISS index
        dimension = embeddings.shape[1]
        self.index = faiss.IndexFlatIP(dimension)  # Inner product for cosine similarity
        
        # Add embeddings to index
        self.index.add(np.array(embeddings, dtype=np.float32))
        
        # Store documents
        self.documents = documents
        
    def retrieve(self, query: str, k: int = 3) -> List[Tuple[str, float]]:
        """Retrieve most relevant documents for a query"""
        # Create query embedding
        query_embedding = self.embedding_model.encode([query])
        
        # Search in FAISS index
        distances, indices = self.index.search(np.array(query_embedding, dtype=np.float32), k)
        
        # Return documents with their similarity scores
        results = []
        for i, (distance, idx) in enumerate(zip(distances[0], indices[0])):
            if idx < len(self.documents):
                results.append((self.documents[idx], distance))
        
        return results
    
    def generate_response(self, query: str, retrieved_docs: List[str]) -> str:
        """Generate a response using retrieved documents"""
        # Create a prompt with retrieved documents
        context = " ".join(retrieved_docs)
        prompt = f"Based on the following information: {context}\n\nAnswer this question: {query}"
        
        # Generate response using the language model
        inputs = self.tokenizer.encode(prompt, return_tensors='pt')
        
        # Generate response
        with torch.no_grad():
            outputs = self.generator.generate(
                inputs,
                max_length=150,
                num_return_sequences=1,
                temperature=0.7,
                do_sample=True,
                pad_token_id=self.tokenizer.eos_token_id
            )
        
        response = self.tokenizer.decode(outputs[0], skip_special_tokens=True)
        return response

# Example usage
if __name__ == "__main__":
    # Initialize RAG system
    rag = AdvancedRAG()
    
    # Sample documents
    documents = [
        "The Eiffel Tower is located in Paris, France.",
        "Albert Einstein was a German theoretical physicist.",
        "The capital of Japan is Tokyo.",
        "Python is a high-level programming language.",
        "The Earth has a circumference of approximately 40,075 kilometers."
    ]
    
    # Add documents to knowledge base
    rag.add_documents(documents)
    
    # Query
    query = "Where is the Eiffel Tower?"
    
    # Retrieve relevant documents
    retrieved = rag.retrieve(query, k=2)
    
    print("Retrieved documents:")
    for doc, score in retrieved:
        print(f"Score: {score:.4f} - {doc}")
    
    # Generate response
    response = rag.generate_response(query, [doc for doc, _ in retrieved])
    print(f"\nResponse: {response}")